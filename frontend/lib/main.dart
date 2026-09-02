import 'dart:async';
import 'dart:io';

import 'package:desktop_drop/desktop_drop.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:hotkey_manager/hotkey_manager.dart';
import 'package:tray_manager/tray_manager.dart';
import 'package:window_manager/window_manager.dart';

import 'core/indexador.dart';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';

import 'core/ffi/frb_generated.dart';
import 'core/theme/app_colors.dart';
import 'core/theme/app_theme.dart';
import 'features/licensing/presentation/providers/license_providers.dart';
import 'features/licensing/presentation/screens/activation_screen.dart';
import 'features/licensing/presentation/screens/license_info_sheet.dart';
import 'features/search/providers/search_provider.dart';
import 'features/fileops/providers/file_ops_provider.dart';
import 'features/fileops/widgets/file_ops_overlay.dart';
import 'features/search/widgets/explorer_bar.dart';
import 'features/search/widgets/status_bar.dart';
import 'features/search/widgets/top_menu_bar.dart';
import 'features/search/widgets/virtualized_table.dart';
import 'features/search/widgets/sidebar_tree.dart';
import 'features/search/widgets/preview_pane.dart';
import 'core/ffi/api.dart' as ffi;

String? _findProjectRoot() {
  var dir = Directory.current;
  for (var i = 0; i < 5; i++) {
    final cargoToml = File('${dir.path}${Platform.pathSeparator}engine${Platform.pathSeparator}Cargo.toml');
    if (cargoToml.existsSync()) {
      return dir.path;
    }
    final parent = dir.parent;
    if (parent.path == dir.path) break;
    dir = parent;
  }
  return null;
}

Future<bool> _compileNativeSync(String root) async {
  try {
    debugPrint('RustLib: ejecutando auto-sanación nativa en $root...');
    final result = Platform.isWindows
        ? await Process.run('powershell', [
            '-ExecutionPolicy',
            'Bypass',
            '-File',
            '$root\\tools\\build_native.ps1',
            '-SkipCodegen',
          ])
        : await Process.run('bash', ['$root/tools/build_macos.sh']);
    return result.exitCode == 0;
  } catch (e) {
    debugPrint('RustLib: error durante compilación nativa: $e');
    return false;
  }
}

Future<void> _initRustLib() async {
  final cwd = Directory.current.path;
  final exeDir = File(Platform.resolvedExecutable).parent.path;

  // 1. Recolectar todas las rutas candidatas
  final candidatePaths = <String>[];
  if (Platform.isWindows) {
    candidatePaths.addAll([
      '$cwd\\..\\engine\\target\\release\\bdj_search_ffi.dll',
      '$cwd\\..\\engine\\target\\debug\\bdj_search_ffi.dll',
      '$cwd\\engine\\target\\release\\bdj_search_ffi.dll',
      '$cwd\\engine\\target\\debug\\bdj_search_ffi.dll',
      '..\\engine\\target\\release\\bdj_search_ffi.dll',
      '..\\engine\\target\\debug\\bdj_search_ffi.dll',
      'engine\\target\\release\\bdj_search_ffi.dll',
      'engine\\target\\debug\\bdj_search_ffi.dll',
      '$exeDir\\bdj_search_ffi.dll',
      '$cwd\\build\\windows\\x64\\runner\\Release\\bdj_search_ffi.dll',
      '$cwd\\build\\windows\\x64\\runner\\Debug\\bdj_search_ffi.dll',
    ]);
  } else if (Platform.isMacOS) {
    candidatePaths.addAll([
      '$cwd/../engine/target/release/libbdj_search_ffi.dylib',
      '$cwd/../engine/target/debug/libbdj_search_ffi.dylib',
      '$cwd/engine/target/release/libbdj_search_ffi.dylib',
      '../engine/target/release/libbdj_search_ffi.dylib',
      '../engine/target/debug/libbdj_search_ffi.dylib',
      '$exeDir/libbdj_search_ffi.dylib',
      '$exeDir/../Frameworks/libbdj_search_ffi.dylib',
      '$cwd/build/macos/Build/Products/Release/libbdj_search_ffi.dylib',
      '$cwd/build/macos/Build/Products/Debug/libbdj_search_ffi.dylib',
    ]);
  }

  // Filtrar solo los archivos existentes en disco
  final existingFiles = candidatePaths
      .map((p) => File(p))
      .where((f) => f.existsSync())
      .toList();

  // Ordenar por fecha de modificación descendente (el binario más reciente primero)
  existingFiles.sort((a, b) {
    try {
      return b.lastModifiedSync().compareTo(a.lastModifiedSync());
    } catch (_) {
      return 0;
    }
  });

  // Auto-sincronización a la carpeta del ejecutable si hay un binario más nuevo
  if (existingFiles.isNotEmpty) {
    final newest = existingFiles.first;
    final targetInExe = File(Platform.isWindows
        ? '$exeDir\\bdj_search_ffi.dll'
        : '$exeDir/libbdj_search_ffi.dylib');
    try {
      if (newest.path != targetInExe.path &&
          (!targetInExe.existsSync() || newest.lastModifiedSync().isAfter(targetInExe.lastModifiedSync()))) {
        newest.copySync(targetInExe.path);
        debugPrint('RustLib: copiado binario más reciente a ${targetInExe.path}');
      }
    } catch (e) {
      debugPrint('RustLib: nota de sincronización: $e');
    }
  }

  // 2. Probar candidatos en orden de novedad hasta que uno valide el hash
  Object? lastError;
  bool initialized = false;

  for (final file in existingFiles) {
    try {
      final lib = ExternalLibrary.open(file.path);
      await RustLib.init(externalLibrary: lib);
      debugPrint('RustLib: inicializado con éxito desde ${file.path}');
      initialized = true;
      break;
    } catch (e) {
      lastError = e;
      debugPrint('RustLib: descartado candidato ${file.path} ($e)');
    }
  }

  // 3. Si ninguno coincidió y estamos en entorno de desarrollo, auto-sanar compilando
  if (!initialized) {
    debugPrint('RustLib: ningún binario coincide con el hash. Intentando auto-sanación...');
    final rootDir = _findProjectRoot();
    if (rootDir != null) {
      final compiled = await _compileNativeSync(rootDir);
      if (compiled) {
        final releaseDll = File(Platform.isWindows
            ? '$rootDir\\engine\\target\\release\\bdj_search_ffi.dll'
            : '$rootDir/engine/target/release/libbdj_search_ffi.dylib');
        if (releaseDll.existsSync()) {
          try {
            final lib = ExternalLibrary.open(releaseDll.path);
            await RustLib.init(externalLibrary: lib);
            debugPrint('RustLib: inicializado tras auto-sanación desde ${releaseDll.path}');
            initialized = true;
          } catch (e) {
            lastError = e;
          }
        }
      }
    }
  }

  if (!initialized) {
    throw StateError('No se pudo inicializar la biblioteca nativa: $lastError');
  }
  debugPrint('RustLib: inicializado correctamente.');
}

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();

  try {
    await _initRustLib();
  } catch (e) {
    debugPrint('RustLib: error al inicializar el puente FFI: $e');
    runApp(MaterialApp(
      debugShowCheckedModeBanner: false,
      home: _FfiRecoveryScreen(error: e.toString()),
    ));
    return;
  }

  // Un solo indexador, y el recien compilado (mata los zombies previos del
  // arbol de build antes de lanzar el suyo). Se lanza en segundo plano: no
  // bloquea la primera ventana.
  unawaited(garantizarIndexador());

  if (Platform.isWindows || Platform.isMacOS || Platform.isLinux) {
    try {
      await windowManager.ensureInitialized();
      await windowManager.setTitle('BDJ Studio Search Pro');
      await hotKeyManager.unregisterAll();
    } catch (e) {
      debugPrint('Desktop window/hotkey manager init note: $e');
    }
  }

  runApp(const ProviderScope(child: SearchProApp()));
}

/// Pantalla de contingencia y recuperación visual si el motor FFI requiere actualización.
class _FfiRecoveryScreen extends StatefulWidget {
  final String error;
  const _FfiRecoveryScreen({required this.error});

  @override
  State<_FfiRecoveryScreen> createState() => _FfiRecoveryScreenState();
}

class _FfiRecoveryScreenState extends State<_FfiRecoveryScreen> {
  bool _sincronizando = false;
  String? _mensaje;

  Future<void> _reintentar() async {
    setState(() {
      _sincronizando = true;
      _mensaje = 'Sincronizando biblioteca nativa con el motor Rust...';
    });

    final root = _findProjectRoot();
    if (root != null) {
      await _compileNativeSync(root);
    }

    try {
      await _initRustLib();
      if (!mounted) return;
      unawaited(garantizarIndexador());
      if (Platform.isWindows || Platform.isMacOS || Platform.isLinux) {
        try {
          await windowManager.ensureInitialized();
          await windowManager.setTitle('BDJ Studio Search Pro');
          await hotKeyManager.unregisterAll();
        } catch (_) {}
      }
      runApp(const ProviderScope(child: SearchProApp()));
    } catch (e) {
      if (mounted) {
        setState(() {
          _sincronizando = false;
          _mensaje = 'No se pudo sincronizar automáticamente: $e';
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: AppColors.surface,
      body: Center(
        child: Container(
          width: 480,
          padding: const EdgeInsets.all(32),
          decoration: BoxDecoration(
            color: Colors.white,
            borderRadius: BorderRadius.circular(12),
            border: Border.all(color: AppColors.border),
            boxShadow: [
              BoxShadow(
                color: Colors.black.withAlpha(15),
                blurRadius: 16,
                offset: const Offset(0, 4),
              ),
            ],
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              const Icon(Icons.sync_problem_rounded, size: 48, color: AppColors.primary),
              const SizedBox(height: 16),
              const Text(
                'Sincronización de Componentes Requerida',
                style: TextStyle(
                  fontSize: 17,
                  fontWeight: FontWeight.bold,
                  color: AppColors.textPrimary,
                ),
              ),
              const SizedBox(height: 12),
              Text(
                _mensaje ??
                    'La interfaz se ha actualizado y requiere sincronizar la biblioteca binaria de Rust con el código de Dart.',
                textAlign: TextAlign.center,
                style: const TextStyle(
                  fontSize: 13,
                  color: AppColors.textSecondary,
                  height: 1.4,
                ),
              ),
              const SizedBox(height: 24),
              if (_sincronizando)
                const CircularProgressIndicator(color: AppColors.primary, strokeWidth: 2.5)
              else
                ElevatedButton.icon(
                  onPressed: _reintentar,
                  icon: const Icon(Icons.refresh_rounded, size: 18),
                  label: const Text('Sincronizar y Abrir Aplicación'),
                  style: ElevatedButton.styleFrom(
                    backgroundColor: AppColors.primary,
                    foregroundColor: Colors.white,
                    padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 12),
                    shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(6)),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

class SearchProApp extends StatelessWidget {
  const SearchProApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'BDJ Studio Search Pro',
      debugShowCheckedModeBanner: false,
      theme: AppTheme.lightTheme,
      home: const LicenseGate(),
    );
  }
}

/// Portón de licencia: nada de la aplicación se construye antes de que la
/// verificación SPP3 haya terminado con éxito.
///
/// Es deliberado que `SearchHomeScreen` solo exista en la rama `licensed`: su
/// provider abre el índice al construirse, y no queremos que un equipo sin
/// licencia llegue siquiera a mapearlo.
class LicenseGate extends ConsumerStatefulWidget {
  const LicenseGate({super.key});

  @override
  ConsumerState<LicenseGate> createState() => _LicenseGateState();
}

class _LicenseGateState extends ConsumerState<LicenseGate>
    with WidgetsBindingObserver {
  /// Lo mínimo que se mantiene el indicador de carga en pantalla.
  ///
  /// Sin esto, una licencia que se verifica muy rápido deja paso a la pantalla
  /// principal en un par de fotogramas, tiempo en el que la barra de estado
  /// todavía está comprobando el motor y parecería un destello de «error». Un
  /// respiro mínimo hace que el arranque llegue ya con el veredicto del motor
  /// encima, sin destellos.
  static const _splashMinimo = Duration(milliseconds: 700);

  /// Tiempo máximo total que el splash puede estar en pantalla.
  ///
  /// Aunque el motor no responda (servicio caído, WMI bloqueado, etc.) la
  /// aplicación tiene que terminar de arrancar. Pasado este tope se cede el
  /// control a la pantalla principal, que ya se encargará de mostrar el motivo
  /// real del fallo en la barra de estado.
  static const _splashTecho = Duration(milliseconds: 4500);

  Timer? _splashTimer;
  Timer? _splashTechoTimer;
  bool _splashCumplido = false;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _splashTimer = Timer(_splashMinimo, () {
      if (mounted) setState(() => _splashCumplido = true);
    });
    _splashTechoTimer = Timer(_splashTecho, () {
      if (mounted) setState(() => _splashCumplido = true);
    });
  }

  @override
  void dispose() {
    _splashTimer?.cancel();
    _splashTechoTimer?.cancel();
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    // Revalidación al volver del segundo plano: detecta el retroceso de reloj y
    // las licencias caducadas sin necesidad de reiniciar la aplicación.
    if (state == AppLifecycleState.resumed) {
      final notifier = ref.read(licenseProvider.notifier);
      if (ref.read(licenseProvider).loadingState ==
          LicenseLoadingState.licensed) {
        notifier.sync();
      }
    }
  }

  /// Comunica al servicio si hay licencia y hasta cuándo.
  ///
  /// El servicio arranca con el sistema y puede pasar meses sin que nadie abra
  /// la aplicación, así que no puede fiarse de su propia memoria: sin este aviso
  /// no indexa. Es lo que cierra el agujero por el que un servicio elevado
  /// recorría el disco entero sin comprobar nada.
  ///
  /// El envío **se reintenta hasta que el servicio confirma** (devuelve true):
  /// el indexador recién lanzado primero hace su escaneo inicial y solo cuando
  /// acaba crea el pipe. Si el aviso se manda una vez y falla en silencio, ese
  /// daemon se queda sin licencia → su bucle no vigila ni republica →
  /// la generación no avanza, y ningún cambio externo (ni siquiera el refresco
  /// manual) se reflejaría. Esto es lo que rompía el «tiempo real» en la
  /// práctica.
  Future<void> _informarLicenciaAlServicio(LicenseState estado) async {
    if (Platform.environment.containsKey('FLUTTER_TEST')) {
      return;
    }
    final activa = estado.loadingState == LicenseLoadingState.licensed;
    final caduca = estado.expiresAt;
    final hasta = BigInt.from(
      caduca == null ? 0 : caduca.millisecondsSinceEpoch ~/ 1000,
    );

    // Tope alto: un escaneo inicial grande (millones de entradas) puede durar
    // varios minutos en arrancar el pipe. Un intento por segundo es barato.
    const intentosMax = 600;
    var informado = false;
    for (var i = 0; i < intentosMax && !informado; i++) {
      try {
        informado = await ffi.serviceSetLicense(active: activa, expiresAt: hasta);
        if (!informado) {
          await Future<void>.delayed(const Duration(seconds: 1));
        }
      } catch (e) {
        debugPrint('No se pudo informar de la licencia al servicio: $e');
        await Future<void>.delayed(const Duration(seconds: 2));
      }
    }
    if (!informado) {
      debugPrint('No se confirmó la licencia del indexador tras $intentosMax intentos.');
    }
  }

  @override
  Widget build(BuildContext context) {
    final licenseState = ref.watch(licenseProvider);

    // Mientras la licencia se está validando se observa también el motor: si
    // este ya dio un veredicto estable (índice abierto o fallo confirmado),
    // el splash cede el control sin esperar al tiempo mínimo artificial. La
    // barra de estado del HomeScreen ya pintará el motivo real.
    final searchState = ref.watch(searchProvider);
    if (licenseState.loadingState == LicenseLoadingState.licensed &&
        !searchState.engine.isChecking &&
        searchState.engine.isStable &&
        !_splashCumplido) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) setState(() => _splashCumplido = true);
      });
    }

    ref.listen(licenseProvider, (anterior, actual) {
      if (anterior?.loadingState != actual.loadingState ||
          anterior?.expiresAt != actual.expiresAt) {
        _informarLicenciaAlServicio(actual);
      }
    });

    switch (licenseState.loadingState) {
      case LicenseLoadingState.initial:
      case LicenseLoadingState.loading:
        return const _SplashScreen();
      case LicenseLoadingState.licensed:
        // La comprobación del motor ocurre al construirse la pantalla
        // principal; se espera el respiro mínimo para no destellar estados
        // transitorios durante el primer fotograma.
        return _splashCumplido
            ? const SearchHomeScreen()
            : const _SplashScreen();
      case LicenseLoadingState.unlicensed:
      case LicenseLoadingState.error:
        return _splashCumplido
            ? const ActivationScreen()
            : const _SplashScreen();
    }
  }
}

class _SplashScreen extends StatelessWidget {
  const _SplashScreen();

  @override
  Widget build(BuildContext context) {
    return const Scaffold(
      backgroundColor: AppColors.background,
      body: Center(
        child: SizedBox(
          height: 26,
          width: 26,
          child: CircularProgressIndicator(
            strokeWidth: 2.4,
            color: AppColors.primary,
          ),
        ),
      ),
    );
  }
}

/// Atajos Alt + 1..8 de los filtros rápidos.
///
/// No puede ser `const`: `LogicalKeyboardKey` redefine `==` y `hashCode`, y Dart
/// prohíbe esos tipos como clave de un mapa constante. Al ser una constante de
/// nivel de archivo se construye una sola vez, no en cada pulsación.
final Map<LogicalKeyboardKey, String> _quickFilters = {
  LogicalKeyboardKey.digit1: 'Todos',
  LogicalKeyboardKey.digit2: 'Audio',
  LogicalKeyboardKey.digit3: 'Proyectos DJ',
  LogicalKeyboardKey.digit4: 'Vídeo',
  LogicalKeyboardKey.digit5: 'Imagen',
  LogicalKeyboardKey.digit6: 'Documentos',
  LogicalKeyboardKey.digit7: 'Comprimidos',
  LogicalKeyboardKey.digit8: 'Carpetas',
};

class SearchHomeScreen extends ConsumerStatefulWidget {
  const SearchHomeScreen({super.key});

  @override
  ConsumerState<SearchHomeScreen> createState() => _SearchHomeScreenState();
}

class _SearchHomeScreenState extends ConsumerState<SearchHomeScreen>
    with TrayListener, WindowListener {
  final TextEditingController _queryController = TextEditingController();
  final FocusNode _searchFocusNode = FocusNode();
  /// Foco del receptor de teclado.
  ///
  /// Antes se construía un `FocusNode` nuevo **dentro de `build`**: uno por
  /// repintado, ninguno liberado.
  final FocusNode _keyboardFocusNode = FocusNode();
  bool _isDragging = false;

  @override
  void initState() {
    super.initState();
    trayManager.addListener(this);
    windowManager.addListener(this);
    _initDesktopServices();
  }

  @override
  void dispose() {
    trayManager.removeListener(this);
    windowManager.removeListener(this);
    _queryController.dispose();
    _searchFocusNode.dispose();
    _keyboardFocusNode.dispose();
    super.dispose();
  }

  @override
  void onWindowClose() {
    // Cuando el usuario le da a la 'X', ocultamos en vez de salir.
    windowManager.hide();
  }

  Future<void> _initDesktopServices() async {
    if (!Platform.isWindows && !Platform.isMacOS && !Platform.isLinux) return;

    try {
      // Atajo global: Alt + Espacio para invocar Search Pro al primer plano
      final hotKey = HotKey(
        key: PhysicalKeyboardKey.space,
        modifiers: [HotKeyModifier.alt],
        scope: HotKeyScope.system,
      );
      await hotKeyManager.register(
        hotKey,
        keyDownHandler: (hotKey) async {
          final isFocused = await windowManager.isFocused();
          if (isFocused) {
            await windowManager.hide();
          } else {
            await windowManager.show();
            await windowManager.focus();
            _searchFocusNode.requestFocus();
          }
        },
      );
    } catch (e) {
      debugPrint('HotKey init note: $e');
    }

    try {
      final menu = Menu(
        items: [
          MenuItem(
            key: 'show_window',
            label: 'Abrir BDJ Studio Search Pro',
          ),
          MenuItem.separator(),
          MenuItem(
            key: 'exit_app',
            label: 'Salir',
          ),
        ],
      );
      await trayManager.setContextMenu(menu);
      
      final String iconPath = Platform.isWindows ? 'assets/images/app_icon.ico' : 'assets/images/logo.png';
      await trayManager.setIcon(iconPath);
      await windowManager.setPreventClose(true);
    } catch (e) {
      debugPrint('Tray init note: $e');
    }
  }

  @override
  void onTrayIconMouseDown() {
    windowManager.show();
    windowManager.focus();
  }

  @override
  void onTrayMenuItemClick(MenuItem menuItem) {
    if (menuItem.key == 'show_window') {
      windowManager.show();
      windowManager.focus();
    } else if (menuItem.key == 'exit_app') {
      terminarApp();
    }
  }

  void _handleKeyEvent(KeyEvent event) {
    if (event is! KeyDownEvent) return;

    final notifier = ref.read(searchProvider.notifier);
    final estado = ref.read(searchProvider);
    final ops = ref.read(fileOpsProvider.notifier);
    final opsEstado = ref.read(fileOpsProvider);
    final teclado = HardwareKeyboard.instance;
    final isAlt = teclado.isAltPressed;
    final isControl = teclado.isControlPressed || teclado.isMetaPressed;
    final isShift = teclado.isShiftPressed;

    // Alt + 1..8: filtros rápidos de categoría.
    if (isAlt) {
      final filter = _quickFilters[event.logicalKey];
      if (filter != null) {
        notifier.setFilter(filter);
        return;
      }
    }

    // Nada de `switch` sobre `LogicalKeyboardKey`: redefine `==`, y Dart
    // prohíbe usar como caso constante un tipo que lo haga.
    final tecla = event.logicalKey;

    // Si el foco está en un campo de texto (buscador, barra de ruta con el
    // lápiz activo, un diálogo de renombrar…), los atajos de la tabla no deben
    // secuestrarlo: Ctrl+V pega la ruta que se está escribiendo, Ctrl+C copia
    // el texto seleccionado, las flechas mueven el cursor… Solo se reservan
    // Esc (cerrar/limpiar), F5 (refresco) y Ctrl+F (volver al buscador).
    if (_escribeEnCampoDeTexto() &&
        !(tecla == LogicalKeyboardKey.escape ||
          tecla == LogicalKeyboardKey.f5 ||
          tecla == LogicalKeyboardKey.keyF)) {
      return;
    }

    if (isControl) {
      if (tecla == LogicalKeyboardKey.keyN) {
        Process.start(Platform.resolvedExecutable, []);
        return;
      }
      if (tecla == LogicalKeyboardKey.keyA) {
        notifier.selectAll();
        return;
      }
      if (tecla == LogicalKeyboardKey.keyC) {
        if (isShift) {
          notifier.copySelectedPath();
          _aviso('Ruta copiada al portapapeles');
        } else if (estado.selection.isEmpty) {
          _aviso('Selecciona algo para copiarlo');
        } else {
          // Como en el Explorador: Copiar colecciona las rutas seleccionadas
          // para un siguiente Pegar, no pega texto suelto en el portapapeles
          // del sistema. La ruta exacta queda en «Copiar la ruta».
          ops.copySelection();
          _aviso('${estado.selection.count} seleccionados, listos para copiar');
        }
        return;
      }
      if (tecla == LogicalKeyboardKey.keyF) {
        // Ctrl+F pone el cursor en la caja y nada más.
        //
        // Antes cambiaba de modo en el acto, así que con la caja vacía dejaba
        // la pantalla en blanco: pulsar «buscar» te quitaba de delante el
        // contenido de la carpeta antes de haber escrito una sola letra. El
        // cambio de modo lo hace la primera pulsación de tecla.
        _searchFocusNode.requestFocus();
        return;
      }
      if (tecla == LogicalKeyboardKey.keyD) {
        notifier.clearSelection();
        return;
      }
      if (tecla == LogicalKeyboardKey.keyI) {
        notifier.invertSelection();
        return;
      }
      if (tecla == LogicalKeyboardKey.keyX) {
        ops.cutSelection();
        _aviso('Listo para mover');
        return;
      }
      if (tecla == LogicalKeyboardKey.keyV) {
        if (estado.mode == ViewMode.browse && opsEstado.clipboard.isNotEmpty) {
          ops.pasteInto(estado.browsePath);
        } else {
          _aviso('Abre una carpeta para pegar dentro de ella');
        }
        return;
      }
      if (tecla == LogicalKeyboardKey.keyZ) {
        // Ctrl+Shift+Z rehace; Ctrl+Z deshace.
        if (isShift) {
          if (opsEstado.canRedo) {
            ops.redoLast();
            _aviso('Rehaciendo la última operación');
          } else {
            _aviso('No hay nada que se pueda rehacer');
          }
        } else if (opsEstado.canUndo) {
          ops.undoLast();
          _aviso('Deshaciendo la última operación');
        } else {
          _aviso('No hay nada que se pueda deshacer');
        }
        return;
      }
    }

    // F5: refresco manual de la vista actual (también está en el menú
    // «Archivo > Actualizar»).
    if (tecla == LogicalKeyboardKey.f5) {
      notifier.refreshNow();
      _aviso('Actualizado');
      return;
    }

    // Suprimir envía a la papelera. Nunca borra de forma definitiva, ni
    // siquiera con Mayús: lo que se borra sin remedio no se puede deshacer, y
    // perder una sesión de trabajo por un atajo no compensa.
    //
    // Se confirma igual que desde el menú: la papelera da la vuelta, pero un
    // golpe de tecla no debe mandar el trabajo de una sesión sin preguntar.
    if (tecla == LogicalKeyboardKey.delete && estado.selection.isNotEmpty) {
      _confirmarYEnviarAPapelera(ops, estado.selection.count);
      return;
    }

    if (tecla == LogicalKeyboardKey.arrowDown) {
      notifier.moveCursor(1, extend: isShift);
    } else if (tecla == LogicalKeyboardKey.arrowUp) {
      notifier.moveCursor(-1, extend: isShift);
    } else if (tecla == LogicalKeyboardKey.pageDown) {
      notifier.moveCursor(20, extend: isShift);
    } else if (tecla == LogicalKeyboardKey.pageUp) {
      notifier.moveCursor(-20, extend: isShift);
    } else if (tecla == LogicalKeyboardKey.home) {
      notifier.moveCursor(-estado.rowCount, extend: isShift);
    } else if (tecla == LogicalKeyboardKey.end) {
      notifier.moveCursor(estado.rowCount, extend: isShift);
    } else if (tecla == LogicalKeyboardKey.enter) {
      // Abrir. Una carpeta se abre dentro de la aplicación.
      notifier.openSelected();
    } else if (tecla == LogicalKeyboardKey.backspace) {
      // Subir un nivel, pero solo si no se está escribiendo en el buscador.
      if (!_searchFocusNode.hasFocus && estado.mode == ViewMode.browse) {
        notifier.goUp();
      }
    } else if (tecla == LogicalKeyboardKey.escape) {
      if (_queryController.text.isNotEmpty) {
        _queryController.clear();
        notifier.setQuery('');
      } else {
        notifier.clearSelection();
      }
    }
  }

  /// ¿El foco está dentro de un campo de texto editable?
  bool _escribeEnCampoDeTexto() {
    final foco = FocusManager.instance.primaryFocus;
    if (foco == null || foco.context == null) return false;
    return foco.context!.findAncestorStateOfType<EditableTextState>() != null;
  }

  /// Enter en el buscador con un texto que parece una ruta la abre, como
  /// haría la barra de direcciones del Explorador al pegar una ruta completa.
  Future<void> _abrirComoRutaSiLoEs(String texto) async {
    final p = texto.trim();
    if (p.isEmpty) return;
    final pareceRuta = p.contains(Platform.pathSeparator) ||
        (Platform.isWindows && RegExp(r'^[A-Za-z]:').hasMatch(p)) ||
        p == '/' ||
        p == '~' ||
        p.startsWith('~/') ||
        p.startsWith('~\\');
    if (!pareceRuta) return;

    var destino = p;
    if (p == '~') destino = _carpetaPersonal;
    if (p.startsWith('~/') || p.startsWith('~\\')) {
      destino = _carpetaPersonal + p.substring(1);
    }
    try {
      final tipo = FileSystemEntity.typeSync(destino);
      if (tipo == FileSystemEntityType.notFound) return;
      if (tipo == FileSystemEntityType.file) {
        // Enter sobre un archivo muestra su carpeta, como en el Explorador.
        destino = File(destino).parent.path;
      }
      await ref.read(searchProvider.notifier).openFolder(destino);
    } catch (_) {}
  }

  String get _carpetaPersonal {
    final env = Platform.environment;
    final h = Platform.isWindows ? env['USERPROFILE'] : env['HOME'];
    if (h != null && h.isNotEmpty) return h;
    return Platform.isWindows ? 'C:\\Users\\' : '/';
  }

  void _aviso(String texto) {
    if (!mounted) return;
    ScaffoldMessenger.of(context)
      ..hideCurrentSnackBar()
      ..showSnackBar(
        SnackBar(content: Text(texto), duration: const Duration(milliseconds: 1200)),
      );
  }

  Future<void> _confirmarYEnviarAPapelera(
    FileOpsNotifier ops,
    int cuantos,
  ) async {
    final confirmado = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text('Enviar a la papelera'),
        content: Text(
          cuantos == 1
              ? '¿Enviar el elemento seleccionado a la papelera?'
              : '¿Enviar $cuantos elementos a la papelera?',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(ctx).pop(false),
            child: const Text('Cancelar'),
          ),
          TextButton(
            onPressed: () => Navigator.of(ctx).pop(true),
            child: const Text('Enviar a la papelera'),
          ),
        ],
      ),
    );
    if (confirmado != true) return;
    await ops.trashSelection();
  }

  @override
  Widget build(BuildContext context) {
    // Solo la carpeta abierta, no el estado entero.
    //
    // Observar `searchProvider` completo aquí reconstruiría toda la pantalla
    // cada vez que la caché de filas se mueve —es decir, en cada
    // desplazamiento—, y esta rama contiene la tabla.
    final carpetaActual = ref.watch(searchProvider.select((s) => s.browsePath));

    // La caja de búsqueda tiene que reflejar el estado, no solo alimentarlo.
    //
    // La búsqueda ahora se puede abandonar desde la barra del explorador
    // —«Salir de la búsqueda»— y no solo borrando el texto a mano. Sin esto el
    // estado quedaba sin consulta pero la caja seguía enseñando lo escrito: se
    // veía el contenido de una carpeta con un texto de búsqueda delante, sin
    // saber cuál de las dos cosas mandaba.
    ref.listen(searchProvider, (anterior, actual) {
      if (actual.query.isEmpty && _queryController.text.isNotEmpty) {
        _queryController.clear();
      }
    });

    return KeyboardListener(
      focusNode: _keyboardFocusNode,
      autofocus: true,
      onKeyEvent: _handleKeyEvent,
      child: Scaffold(
        backgroundColor: AppColors.background,
        body: Column(
          children: [
            const TopMenuBar(),
            Container(
              padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
              decoration: const BoxDecoration(
                color: AppColors.surface,
                border: Border(bottom: BorderSide(color: AppColors.border)),
              ),
              child: Row(
                children: [
                  ClipRRect(
                    borderRadius: BorderRadius.circular(6),
                    child: Image.asset(
                      'assets/images/logo.png',
                      width: 24,
                      height: 24,
                      fit: BoxFit.cover,
                    ),
                  ),
                  const SizedBox(width: 10),
                  const Icon(Icons.search_rounded,
                      color: AppColors.primary, size: 22),
                  const SizedBox(width: 12),
                  Expanded(
                    child: TextField(
                      controller: _queryController,
                      focusNode: _searchFocusNode,
                      autofocus: true,
                      style: const TextStyle(
                        fontSize: 15,
                        color: AppColors.textPrimary,
                        fontWeight: FontWeight.w500,
                      ),
                      decoration: InputDecoration(
                        // El texto de ayuda dice **dónde** va a buscar.
                        //
                        // Escribir filtra la carpeta abierta, igual que en el
                        // Explorador; si no se dice, el usuario supone que
                        // busca en todo el equipo y no entiende por qué no
                        // aparece un archivo que sabe que tiene.
                        hintText: carpetaActual.isEmpty
                            ? 'Buscar en todo el equipo '
                                '(ej: michael jackson ext:wav tam:>10mb)...'
                            : 'Buscar en $carpetaActual…',
                        hintStyle: const TextStyle(
                            color: AppColors.textDisabled, fontSize: 13),
                        border: InputBorder.none,
                        isDense: true,
                      ),
                      onChanged: (val) {
                        // Con retardo: escribir de corrido ya no lanza una
                        // consulta por tecla.
                        ref.read(searchProvider.notifier).setQuery(val);
                      },
                      onSubmitted: _abrirComoRutaSiLoEs,
                    ),
                  ),
                  if (_queryController.text.isNotEmpty)
                    IconButton(
                      icon: const Icon(Icons.clear,
                          size: 18, color: AppColors.textSecondary),
                      onPressed: () {
                        _queryController.clear();
                        ref.read(searchProvider.notifier).setQuery('');
                      },
                    ),
                  const SizedBox(width: 4),
                  IconButton(
                    icon: const Icon(Icons.verified_user_outlined,
                        size: 18, color: AppColors.textSecondary),
                    tooltip: 'Licencia',
                    onPressed: () => LicenseInfoSheet.show(context),
                  ),
                ],
              ),
            ),
            Expanded(
              child: DropTarget(
                onDragDone: (detail) {
                  if (detail.files.isEmpty) return;
                  final path = detail.files.first.path;
                  // Soltar una carpeta la abre; soltar un archivo muestra su
                  // carpeta contenedora. Antes ambos casos escribían una
                  // consulta `ruta:` en el buscador, que es más lento y menos
                  // útil que entrar directamente.
                  ref.read(searchProvider.notifier).openFolder(path);
                },
                onDragEntered: (_) => setState(() => _isDragging = true),
                onDragExited: (_) => setState(() => _isDragging = false),
                child: Stack(
                  children: [
                    const Column(
                      children: [
                        ExplorerBar(),
                        Expanded(
                          child: Row(
                            children: [
                              SidebarTree(),
                              Expanded(child: VirtualizedTable()),
                              PreviewPane(),
                            ],
                          ),
                        ),
                      ],
                    ),
                    const FileOpsOverlay(),
                    if (_isDragging)
                      Container(
                        color: AppColors.primary.withAlpha(40),
                        child: Center(
                          child: Container(
                            padding: const EdgeInsets.symmetric(
                                horizontal: 24, vertical: 16),
                            decoration: BoxDecoration(
                              color: AppColors.surface,
                              borderRadius: BorderRadius.circular(8),
                              border: Border.all(
                                  color: AppColors.primary, width: 2),
                            ),
                            child: const Row(
                              mainAxisSize: MainAxisSize.min,
                              children: [
                                Icon(Icons.folder_open_rounded,
                                    color: AppColors.primary, size: 24),
                                SizedBox(width: 12),
                                Text(
                                  'Soltar carpeta para buscar dentro de ella',
                                  style: TextStyle(
                                    fontSize: 14,
                                    fontWeight: FontWeight.w600,
                                    color: AppColors.textPrimary,
                                  ),
                                ),
                              ],
                            ),
                          ),
                        ),
                      ),
                  ],
                ),
              ),
            ),
            const SearchStatusBar(),
          ],
        ),
      ),
    );
  }
}
