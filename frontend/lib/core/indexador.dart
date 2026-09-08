import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';

import 'ffi/api.dart' as ffi;

/// El indexador que esta sesion ha lanzado, para poder pararlo al cerrar.
///
/// Solo importa el que lanzamos nosotros (`--standalone`). Un servicio
/// instalado del sistema no pasa por aqui y no se toca.
Process? _indexadorPropio;

/// Refresca la foto del indexador que tenemos lanzado.
void _registrarIndexadorPropio(Process p) => _indexadorPropio = p;

/// Garantiza que haya exactamente UN indexador, y que sea el binario recien
/// compilado de esta sesion.
///
/// Antes se lanzaba `--standalone` *a ciegas* (detached) sin mirar los que ya
/// quedaran de sesiones anteriores. Eso dejaba zombies que:
///   - bloqueaban la sobrescritura de `bdj_search_indexer.exe` en el build
///     («El proceso no puede obtener acceso… en uso») y
///   - creaban DOS indexadores compitiendo por el mismo pipe/IPC y por el mismo
///     `index.bdjx`. La app mandaba la licencia y los cambios al dueno del
///     pipe, pero leia un indice que podia apara con una generacion congelada:
///     renombrar un archivo desde el Explorador nunca se reflejaba.
///
/// Aqui se mata primero cualquier indexador que viva DENTRO del arbol de build
/// de esta aplicacion y despues se lanza el binario actual en exclusiva.
Future<void> garantizarIndexador() async {
  if (!Platform.isWindows && !Platform.isMacOS && !Platform.isLinux) return;
  try {
    // Si el servicio del sistema ya está activo y respondiendo, no lanzar otro.
    try {
      final s = await ffi.serviceStatus();
      if (s.reachable) {
        debugPrint('Servicio indexador del sistema ya activo y respondiendo.');
        return;
      }
    } catch (_) {}

    if (!kReleaseMode) {
      await _mataZombisDeSesionesAnteriores();
    }
    final exe = _buscarBinario();
    if (exe == null) {
      debugPrint('(no se encontro el binario del indexador)');
      return;
    }
    debugPrint('Asegurando indexador en segundo plano: $exe');
    final p = await Process.start(
      exe,
      ['--standalone'],
      mode: ProcessStartMode.detached,
    );
    _registrarIndexadorPropio(p);
  } catch (e) {
    debugPrint('Indexer spawn note: $e');
  }
}

/// Mata los indexadores de sesiones anteriores.
///
/// Solo mira los que viven dentro del arbol de build de esta aplicacion
/// (`frontend\build\windows\...`, `build/macos/Build/Products/...` o
/// `engine/target/...`). Un servicio del sistema instalado usa otro directorio
/// y se respeta.
Future<void> _mataZombisDeSesionesAnteriores() async {
  if (kReleaseMode) return;
  try {
    if (Platform.isWindows) {
      // Get-Process por nombre + filtro por ruta: no hace falta taskkill
      // global, que tiraria tambien un servicio real con el mismo nombre.
      const ps =
          "Get-Process bdj_search_indexer* -ErrorAction SilentlyContinue | "
          "Where-Object { \$_.Path -like '*frontend*build*windows*' } | "
          "ForEach-Object { Stop-Process -Id \$_.Id -Force -ErrorAction SilentlyContinue }";
      await Process.run('powershell', ['-NoProfile', '-Command', ps]);
    } else {
      for (final pat in [
        'build/macos/Build/Products',
        'engine/target/debug/bdj_search_indexer',
        'engine/target/release/bdj_search_indexer',
      ]) {
        await Process.run('pkill', ['-f', pat]);
      }
    }
    // Dar tiempo a que suelten el binario y cierren el index.bdjx.
    await Future<void>.delayed(const Duration(milliseconds: 300));
  } catch (_) {}
}

String? _buscarBinario() {
  final exeDir = File(Platform.resolvedExecutable).parent.path;
  final cwd = Directory.current.path;
  final candidates = <String>[];
  if (Platform.isWindows) {
    candidates.addAll([
      '$exeDir\\bdj_search_indexer.exe',
      '..\\engine\\target\\release\\bdj_search_indexer.exe',
      '..\\engine\\target\\debug\\bdj_search_indexer.exe',
      'engine\\target\\release\\bdj_search_indexer.exe',
      'engine\\target\\debug\\bdj_search_indexer.exe',
    ]);
  } else {
    candidates.addAll([
      '$exeDir/bdj_search_indexer',
      '$cwd/build/macos/Build/Products/Debug/bdj_search_indexer',
      '$cwd/build/macos/Build/Products/Release/bdj_search_indexer',
      '../engine/target/release/bdj_search_indexer',
      '../engine/target/debug/bdj_search_indexer',
      'engine/target/release/bdj_search_indexer',
      'engine/target/debug/bdj_search_indexer',
    ]);
  }
  for (final c in candidates) {
    if (File(c).existsSync()) return c;
  }
  return null;
}

/// Candado que garantiza **una sola instancia** de la aplicación.
///
/// Se mantiene abierto mientras el programa vive; el sistema lo suelta solo al
/// terminar el proceso, incluso si se cierra de golpe.
RandomAccessFile? _candado;

/// ¿Es esta la única instancia?
///
/// Devuelve `false` cuando ya hay otra corriendo, y entonces esta debe salir.
///
/// # Por qué hace falta
///
/// El usuario veía tres iconos de la aplicación en la barra de tareas. Uno era
/// la consola del servicio —ya corregido, el servicio ya no abre ventana— y los
/// otros dos eran dos instancias de verdad. Dos instancias no son solo dos
/// iconos: cada una lanza su propio indexador al arrancar y mata el de la
/// anterior, así que se turnan matándose, y las dos leen y escriben el mismo
/// `index.bdjx`. El síntoma era un índice que a veces se quedaba congelado.
///
/// El candado es un archivo bloqueado en exclusiva junto al índice. No hace
/// falta contar procesos ni buscarlos por nombre: si el bloqueo se puede tomar,
/// no hay nadie más; si no, sí lo hay.
ServerSocket? _ipcServer;
void Function()? _onWakeRequested;

/// Registra la acción que debe ejecutarse cuando se solicita despertar la ventana.
void registrarActivadorInstancia(void Function() callback) {
  _onWakeRequested = callback;
}

/// Inicia el servidor IPC local en la instancia activa para escuchar órdenes de activación.
Future<void> _iniciarServidorInstancia() async {
  try {
    _ipcServer = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
    final port = _ipcServer!.port;
    final portFile = File('${Directory.systemTemp.path}${Platform.pathSeparator}bdj_search_pro.port');
    await portFile.writeAsString('$port');
    _ipcServer!.listen((socket) {
      socket.listen((data) {
        final msg = utf8.decode(data, allowMalformed: true).trim();
        if (msg.contains('SHOW_WINDOW')) {
          _onWakeRequested?.call();
        }
      });
    });
  } catch (e) {
    debugPrint('Nota servidor IPC instancia: $e');
  }
}

Future<bool> reclamarInstanciaUnica() async {
  try {
    final dir = Directory.systemTemp;
    final f = File('${dir.path}${Platform.pathSeparator}bdj_search_pro.lock');
    final raf = await f.open(mode: FileMode.write);
    try {
      // Sin esperar: si otro lo tiene, se sabe al instante.
      raf.lockSync(FileLock.exclusive);
    } on FileSystemException {
      await raf.close();
      await _activarInstanciaExistente();
      return false;
    }
    _candado = raf;
    await _iniciarServidorInstancia();
    return true;
  } catch (e) {
    if (e is FileSystemException) {
      // En Windows, abrir un archivo ya abierto en modo exclusivo lanza
      // FileSystemException (errno 32: ERROR_SHARING_VIOLATION). Eso significa
      // inequívocamente que ya hay otra instancia viva en ejecución.
      debugPrint('Instancia previa detectada: $e');
      await _activarInstanciaExistente();
      return false;
    }
    debugPrint('Nota de candado de instancia única: $e');
    await _iniciarServidorInstancia();
    return true;
  }
}

Future<void> _activarInstanciaExistente() async {
  // 1. Despertar la instancia viva vía socket local (instantáneo, infalible incluso si la ventana está oculta)
  try {
    final portFile = File('${Directory.systemTemp.path}${Platform.pathSeparator}bdj_search_pro.port');
    if (await portFile.exists()) {
      final portStr = (await portFile.readAsString()).trim();
      final port = int.tryParse(portStr);
      if (port != null) {
        final socket = await Socket.connect(
          InternetAddress.loopbackIPv4,
          port,
          timeout: const Duration(milliseconds: 700),
        );
        socket.write('SHOW_WINDOW\n');
        await socket.flush();
        await socket.close();
        return;
      }
    }
  } catch (e) {
    debugPrint('Nota comunicando con instancia existente: $e');
  }

  // 2. Respaldo por herramientas del sistema si el socket no respondiera
  if (Platform.isWindows) {
    try {
      Process.run('powershell', [
        '-NoProfile',
        '-Command',
        r'(New-Object -ComObject WScript.Shell).AppActivate("BDJ Studio Search Pro")'
      ]);
    } catch (_) {}
  } else if (Platform.isMacOS) {
    try {
      Process.run('osascript', [
        '-e',
        'tell application "BDJ Studio Search Pro" to activate',
      ]);
    } catch (_) {}
  }
}

/// Suelta el candado de instancia única y cierra el servidor IPC.
Future<void> soltarInstanciaUnica() async {
  try {
    await _ipcServer?.close();
    _ipcServer = null;
  } catch (_) {}
  try {
    final portFile = File('${Directory.systemTemp.path}${Platform.pathSeparator}bdj_search_pro.port');
    if (await portFile.exists()) {
      await portFile.delete();
    }
  } catch (_) {}
  final raf = _candado;
  _candado = null;
  if (raf == null) return;
  try {
    raf.unlockSync();
    await raf.close();
  } catch (_) {}
}

/// Detiene el indexador que lanzo esta sesion.
///
/// Se llama al salir de la aplicacion: asi el `.exe` del Debug deja de estar
/// «en uso» y se puede recompilar sin mensajes de acceso denegado, y nunca
/// quedan daemons huerfanos conteniendo el indice.
Future<void> detenerIndexadorPropio() async {
  final p = _indexadorPropio;
  _indexadorPropio = null;
  if (p == null) return;
  try {
    p.kill();
  } catch (_) {}
}

/// Salida limpia: detiene el indexador de la sesion y termina el proceso.
Future<void> terminarApp() async {
  try {
    await detenerIndexadorPropio();
  } catch (_) {}
  try {
    await soltarInstanciaUnica();
  } catch (_) {}
  exit(0);
}

/// Abre una ventana nueva del explorador.
void abrirNuevaVentana() {
  try {
    Process.start(Platform.resolvedExecutable, ['--new-window'], mode: ProcessStartMode.detached);
  } catch (e) {
    debugPrint('No se pudo abrir nueva ventana: $e');
  }
}