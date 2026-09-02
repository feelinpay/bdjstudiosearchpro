import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';

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
    await _mataZombisDeSesionesAnteriores();
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
  exit(0);
}