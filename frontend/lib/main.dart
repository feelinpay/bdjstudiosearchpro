import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:desktop_drop/desktop_drop.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:hotkey_manager/hotkey_manager.dart';
import 'package:tray_manager/tray_manager.dart';
import 'package:window_manager/window_manager.dart';

import 'package:shared_preferences/shared_preferences.dart';
import 'core/i18n/app_strings.dart';
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
import 'features/fileops/file_ops_dialogs.dart';
import 'features/fileops/widgets/file_ops_overlay.dart';
import 'features/search/widgets/explorer_bar.dart';
import 'features/search/widgets/status_bar.dart';
import 'features/search/widgets/top_menu_bar.dart';
import 'features/search/widgets/virtualized_table.dart';
import 'features/search/widgets/sidebar_tree.dart';
import 'features/search/widgets/preview_pane.dart';
import 'features/search/providers/preview_provider.dart';
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

/// ¿Estamos dentro del árbol del proyecto, o es una instalación de verdad?
///
/// Cambia lo único que importa cuando el motor no carga: a quién se lo estamos
/// contando. En el equipo de quien desarrolla, la respuesta útil son los dos
/// comandos que hay que ejecutar. En el equipo de un DJ, hablarle de `cargo` y
/// de «content hash» no es información: es ruido, y encima en inglés.
bool _esEntornoDeDesarrollo() => _findProjectRoot() != null;

Future<void> _initRustLib() async {
  final cwd = Directory.current.path;
  final exeDir = File(Platform.resolvedExecutable).parent.path;

  // En modo release instalado, cargar directamente la DLL que viaja junto al ejecutable (< 1 ms).
  final prodDll = File(Platform.isWindows
      ? '$exeDir\\bdj_search_ffi.dll'
      : (Platform.isMacOS ? '$exeDir/libbdj_search_ffi.dylib' : ''));
  if (kReleaseMode && prodDll.existsSync()) {
    try {
      final lib = ExternalLibrary.open(prodDll.path);
      await RustLib.init(externalLibrary: lib);
      debugPrint('RustLib: inicializado instantáneamente desde ${prodDll.path}');
      return;
    } catch (_) {}
  }

  // 1. Recolectar todas las rutas candidatas (modo desarrollo)
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

  // Si ninguno cargó, se acabó. **La aplicación no compila nada.**
  //
  // Antes lanzaba `cargo` desde aquí para «auto-sanarse». Eso está mal por tres
  // motivos, y los tres se vieron a la vez: dejaba el arranque compilando en
  // bucle —CPU y memoria al ochenta por ciento—, fallaba igual, y en el equipo
  // de un usuario no puede funcionar porque ahí no hay compilador. Un programa
  // instalado no se recompila a sí mismo; se instala bien o avisa.
  if (!initialized) {
    throw StateError('No se pudo inicializar la biblioteca nativa: $lastError');
  }
  debugPrint('RustLib: inicializado correctamente.');
}

// Aquí había una comprobación de contrato propia: el motor declaraba un número
// de versión y la aplicación lo comparaba con el suyo al arrancar.
//
// Sobraba. `flutter_rust_bridge` ya compara un hash del contenido de los
// enlaces, lo hace solo y no depende de que nadie se acuerde de subir un
// número. Mantener dos mecanismos para lo mismo es una cosa más que puede
bool isSecondaryWindow = false;
bool startMinimized = false;

Future<void> main(List<String> args) async {
  WidgetsFlutterBinding.ensureInitialized();

  isSecondaryWindow = args.contains('--new-window');
  startMinimized = args.contains('--startup') ||
      args.contains('--tray') ||
      args.contains('--minimized');

  // Si no es una ventana secundaria abierta desde la app, exigir instancia única.
  if (!isSecondaryWindow && !await reclamarInstanciaUnica()) {
    debugPrint('Ya hay una instancia de BDJ Studio Search Pro en marcha.');
    exit(0);
  }

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
      if (startMinimized) {
        await windowManager.hide();
      }
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
  bool _reintentando = false;
  String? _mensaje;

  /// Vuelve a intentar **cargar** el motor. No compila nada.
  ///
  /// Sirve para el caso real: se compiló el motor en otra ventana y se quiere
  /// abrir sin cerrar y volver a lanzar la aplicación. Cuesta milisegundos.
  Future<void> _reintentar() async {
    setState(() {
      _reintentando = true;
      _mensaje = null;
    });
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
      if (!mounted) return;
      setState(() {
        _reintentando = false;
        _mensaje = 'Sigue sin cargar. $e';
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final desarrollo = _esEntornoDeDesarrollo();

    // El título y la explicación cambian según a quién se le está contando.
    final titulo = desarrollo
        ? 'El motor compilado no coincide con la aplicación'
        : 'Falta un componente de la instalación';

    final explicacion = desarrollo
        ? 'Los enlaces de Dart se regeneraron después de compilar el motor, o al '
              'revés. Compílalo en este orden y vuelve a abrir:'
        : 'BDJ Studio Search Pro no ha podido cargar su motor de búsqueda. '
              'Vuelve a instalar la aplicación; si el problema sigue, escríbenos '
              'y lo resolvemos.';

    return Scaffold(
      backgroundColor: AppColors.surface,
      body: Center(
        child: Container(
          width: 520,
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
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              const Icon(Icons.extension_off_rounded,
                  size: 44, color: AppColors.textSecondary),
              const SizedBox(height: 16),
              Text(
                titulo,
                textAlign: TextAlign.center,
                style: const TextStyle(
                  fontSize: 17,
                  fontWeight: FontWeight.bold,
                  color: AppColors.textPrimary,
                ),
              ),
              const SizedBox(height: 12),
              Text(
                explicacion,
                textAlign: TextAlign.center,
                style: const TextStyle(
                  fontSize: 13,
                  color: AppColors.textSecondary,
                  height: 1.45,
                ),
              ),

              // Los comandos, solo donde sirven de algo.
              if (desarrollo) ...[
                const SizedBox(height: 16),
                Container(
                  padding: const EdgeInsets.all(14),
                  decoration: BoxDecoration(
                    color: AppColors.surfaceAlt,
                    borderRadius: BorderRadius.circular(6),
                    border: Border.all(color: AppColors.border),
                  ),
                  child: const SelectableText(
                    'cd frontend\n'
                    'flutter_rust_bridge_codegen generate\n'
                    '\n'
                    'cd ..\\engine\n'
                    'cargo build --release',
                    style: TextStyle(
                      fontFamily: 'monospace',
                      fontSize: 12.5,
                      height: 1.5,
                      color: AppColors.textPrimary,
                    ),
                  ),
                ),
                const SizedBox(height: 8),
                const Text(
                  'Primero generar, después compilar: el generador escribe código '
                  'Rust que hay que compilar a continuación.',
                  textAlign: TextAlign.center,
                  style: TextStyle(
                    fontSize: 11.5,
                    color: AppColors.textSecondary,
                    fontStyle: FontStyle.italic,
                  ),
                ),
              ],

              if (_mensaje != null) ...[
                const SizedBox(height: 14),
                SelectableText(
                  _mensaje!,
                  textAlign: TextAlign.center,
                  style: const TextStyle(fontSize: 11.5, color: Colors.red),
                ),
              ],

              const SizedBox(height: 22),
              if (_reintentando)
                const Center(
                  child: SizedBox(
                    width: 22,
                    height: 22,
                    child: CircularProgressIndicator(
                        color: AppColors.primary, strokeWidth: 2.5),
                  ),
                )
              else
                ElevatedButton.icon(
                  onPressed: _reintentar,
                  icon: const Icon(Icons.refresh_rounded, size: 18),
                  label: Text(desarrollo
                      ? 'Ya lo he compilado, reintentar'
                      : 'Reintentar'),
                  style: ElevatedButton.styleFrom(
                    backgroundColor: AppColors.primary,
                    foregroundColor: Colors.white,
                    padding: const EdgeInsets.symmetric(vertical: 13),
                    shape: RoundedRectangleBorder(
                        borderRadius: BorderRadius.circular(6)),
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
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) {
      final notifier = ref.read(licenseProvider.notifier);
      if (ref.read(licenseProvider).loadingState ==
          LicenseLoadingState.licensed) {
        notifier.sync();
      }
    }
  }

  /// Persiste el estado de licencia en los ajustes compartidos del indexador
  /// para que arranque conociendo la licencia inmediatamente sin esperar IPC.
  Future<void> _persistirLicenciaEnSettings(bool activa, int hasta) async {
    try {
      final String dirPath;
      if (Platform.isWindows) {
        final progData = Platform.environment['ProgramData'] ?? r'C:\ProgramData';
        dirPath = '$progData\\BDJ Studio\\Search Pro';
      } else if (Platform.isMacOS) {
        final home = Platform.environment['HOME'] ?? '';
        dirPath = '$home/Library/Application Support/BDJ Studio/Search Pro';
      } else {
        return;
      }
      final dir = Directory(dirPath);
      if (!dir.existsSync()) {
        dir.createSync(recursive: true);
      }
      final file = File('$dirPath${Platform.pathSeparator}settings.json');
      Map<String, dynamic> data = {};
      if (file.existsSync()) {
        try {
          data = jsonDecode(file.readAsStringSync()) as Map<String, dynamic>;
        } catch (_) {}
      }
      data['indexing_enabled'] = data['indexing_enabled'] ?? true;
      data['license_active'] = activa;
      data['license_expires_at'] = hasta;
      file.writeAsStringSync(jsonEncode(data));
    } catch (_) {}
  }

  /// Comunica al servicio si hay licencia y hasta cuándo (en segundo plano y sin bloquear).
  Future<void> _informarLicenciaAlServicio(LicenseState estado) async {
    if (Platform.environment.containsKey('FLUTTER_TEST')) {
      return;
    }
    final activa = estado.loadingState == LicenseLoadingState.licensed;
    final caduca = estado.expiresAt;
    final hasta = BigInt.from(
      caduca == null ? 0 : caduca.millisecondsSinceEpoch ~/ 1000,
    );

    // 1. Persistencia directa en disco (leída de inmediato por el daemon al iniciar)
    await _persistirLicenciaEnSettings(activa, hasta.toInt());

    // 2. Notificación en caliente al indexador si ya tiene la tubería/socket abierta
    try {
      await ffi.serviceSetLicense(active: activa, expiresAt: hasta);
    } catch (_) {}
  }

  @override
  Widget build(BuildContext context) {
    final licenseState = ref.watch(licenseProvider);

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
        return const SearchHomeScreen();
      case LicenseLoadingState.unlicensed:
      case LicenseLoadingState.error:
        return const ActivationScreen();
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
  List<String> _recentSearches = [];
  bool _showRecentSearches = false;

  @override
  void initState() {
    super.initState();
    trayManager.addListener(this);
    windowManager.addListener(this);
    registrarActivadorInstancia(() {
      if (mounted) {
        _mostrarVentana();
      }
    });
    _initDesktopServices();
    _setupRecentSearches();
    if (startMinimized) {
      WidgetsBinding.instance.addPostFrameCallback((_) async {
        try {
          await windowManager.hide();
        } catch (_) {}
      });
    }
  }

  void _setupRecentSearches() {
    _loadRecentSearches();
    _searchFocusNode.addListener(() {
      final shouldShow = _searchFocusNode.hasFocus &&
          _queryController.text.trim().isEmpty &&
          _recentSearches.isNotEmpty;
      if (_showRecentSearches != shouldShow && mounted) {
        setState(() => _showRecentSearches = shouldShow);
      }
    });
    _queryController.addListener(() {
      final shouldShow = _searchFocusNode.hasFocus &&
          _queryController.text.trim().isEmpty &&
          _recentSearches.isNotEmpty;
      if (_showRecentSearches != shouldShow && mounted) {
        setState(() => _showRecentSearches = shouldShow);
      }
    });
  }

  Future<void> _loadRecentSearches() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      if (mounted) {
        setState(() {
          _recentSearches = prefs.getStringList('recent_searches') ?? [];
        });
      }
    } catch (_) {}
  }

  Future<void> _addRecentSearch(String q) async {
    final term = q.trim();
    if (term.isEmpty || term.length < 2) return;
    _recentSearches.remove(term);
    _recentSearches.insert(0, term);
    if (_recentSearches.length > 8) {
      _recentSearches = _recentSearches.sublist(0, 8);
    }
    try {
      final prefs = await SharedPreferences.getInstance();
      await prefs.setStringList('recent_searches', _recentSearches);
    } catch (_) {}
    if (mounted) setState(() {});
  }

  Future<void> _clearRecentSearches() async {
    _recentSearches.clear();
    try {
      final prefs = await SharedPreferences.getInstance();
      await prefs.remove('recent_searches');
    } catch (_) {}
    if (mounted) setState(() => _showRecentSearches = false);
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
    if (isSecondaryWindow) {
      windowManager.destroy();
    } else {
      // Cuando el usuario le da a la 'X' en la ventana principal, ocultamos a la bandeja.
      windowManager.hide();
    }
  }

  @override
  void onWindowBlur() {
    // Al perder el foco (hacer clic en otra ventana o en el escritorio),
    // cualquier menú contextual emergente se cierra de inmediato como en el SO nativo.
    VirtualizedTable.dismissActiveMenu();
  }

  Future<void> _initDesktopServices() async {
    if (!Platform.isWindows && !Platform.isMacOS && !Platform.isLinux) return;
    if (isSecondaryWindow) {
      // Las ventanas secundarias solo gestionan su propia ventana y no duplican bandejas ni atajos.
      await windowManager.setPreventClose(false);
      return;
    }

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
            label: 'Mostrar aplicación',
          ),
          MenuItem.separator(),
          MenuItem(
            key: 'exit_app',
            label: 'Salir',
          ),
        ],
      );

      final String iconPath = Platform.isWindows ? 'assets/images/app_icon.ico' : 'assets/images/logo.png';
      // CRÍTICO WINDOWS/MACOS: setIcon DEBE llamarse ANTES de setContextMenu.
      // Si se llama setContextMenu primero, SetIcon() interno reinicializa hMenu
      // con CreatePopupMenu() vacío y el menú contextual no aparece.
      await trayManager.setIcon(iconPath);
      await trayManager.setContextMenu(menu);
      await windowManager.setPreventClose(true);
    } catch (e) {
      debugPrint('Tray init note: $e');
    }
  }

  Future<void> _mostrarVentana() async {
    try {
      if (await windowManager.isMinimized()) {
        await windowManager.restore();
      }
      await windowManager.show();
      await windowManager.focus();
      _searchFocusNode.requestFocus();
    } catch (e) {
      debugPrint('Error al mostrar ventana: $e');
    }
  }

  @override
  void onTrayIconMouseDown() {
    _mostrarVentana();
  }

  @override
  void onTrayIconRightMouseDown() {
    trayManager.popUpContextMenu();
  }

  @override
  void onTrayIconRightMouseUp() {
    trayManager.popUpContextMenu();
  }

  @override
  void onTrayMenuItemClick(MenuItem menuItem) {
    if (menuItem.key == 'show_window') {
      _mostrarVentana();
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

    // Si el foco está en la caja de búsqueda y pulsa Flecha Abajo o Intro:
    if (_searchFocusNode.hasFocus) {
      if (tecla == LogicalKeyboardKey.arrowDown) {
        _searchFocusNode.unfocus();
        if (estado.rowCount > 0) {
          if (estado.selection.isEmpty) {
            notifier.selectRow(0);
          } else {
            notifier.moveCursor(1);
          }
        }
        return;
      }
      if (tecla == LogicalKeyboardKey.enter) {
        final q = _queryController.text.trim();
        final pareceRuta = q.contains(Platform.pathSeparator) ||
            (Platform.isWindows && RegExp(r'^[A-Za-z]:').hasMatch(q)) ||
            q == '/' || q == '~' || q.startsWith('~/') || q.startsWith('~\\');
        if (pareceRuta) {
          _abrirComoRutaSiLoEs(q);
          return;
        }
        if (q.isNotEmpty) {
          notifier.forceSearch(q);
          return;
        }
        if (estado.rowCount > 0) {
          if (estado.selection.isEmpty) {
            notifier.selectRow(0);
          }
          notifier.openSelected();
        }
        return;
      }
    }

    if (_escribeEnCampoDeTexto() &&
        !(tecla == LogicalKeyboardKey.escape ||
          tecla == LogicalKeyboardKey.f5 ||
          tecla == LogicalKeyboardKey.keyF)) {
      return;
    }

    if (isControl) {
      if (tecla == LogicalKeyboardKey.keyN) {
        if (isShift) {
          if (estado.mode == ViewMode.browse) {
            nuevaCarpetaDialog(context, ops, estado.browsePath);
          } else {
            _aviso('Navega a una carpeta para crear una nueva dentro');
          }
        } else {
          // Ctrl+N: Nueva búsqueda limpia
          _queryController.clear();
          notifier.setQuery('');
          _searchFocusNode.requestFocus();
        }
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
        _searchFocusNode.requestFocus();
        return;
      }
      if (tecla == LogicalKeyboardKey.keyD) {
        if (isShift) {
          if (estado.selection.isNotEmpty) {
            ops.duplicateSelection();
            _aviso('Duplicando elementos seleccionados');
          }
        } else {
          notifier.clearSelection();
        }
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
      if (tecla == LogicalKeyboardKey.keyY) {
        if (opsEstado.canRedo) {
          ops.redoLast();
          _aviso('Rehaciendo la última operación');
        } else {
          _aviso('No hay nada que se pueda rehacer');
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

    if (isAlt) {
      if (tecla == LogicalKeyboardKey.arrowLeft && estado.canGoBack) {
        notifier.goBack();
        return;
      }
      if (tecla == LogicalKeyboardKey.arrowRight && estado.canGoForward) {
        notifier.goForward();
        return;
      }
      if (tecla == LogicalKeyboardKey.arrowUp && estado.mode == ViewMode.browse) {
        notifier.goUp();
        return;
      }
      if (tecla == LogicalKeyboardKey.keyP) {
        ref.read(previewProvider.notifier).toggle();
        return;
      }
    }

    // F2: Cambiar nombre del elemento seleccionado
    if (tecla == LogicalKeyboardKey.f2 && estado.selection.isNotEmpty) {
      renombrarDialog(context, ref, ops);
      return;
    }

    // F5: refresco manual de la vista actual (también está en el menú
    // «Archivo > Actualizar»).
    if (tecla == LogicalKeyboardKey.f5) {
      notifier.refreshNow();
      _aviso('Actualizado');
      return;
    }

    // Suprimir envía a la papelera (reversible).
    // Con Shift + Suprimir se elimina definitivamente (irreversible).
    if (tecla == LogicalKeyboardKey.delete && estado.selection.isNotEmpty) {
      if (isShift) {
        eliminarPermanentemente(context, ops, estado.selection.count);
      } else {
        confirmarEnviarALaPapelera(context, ops, estado.selection.count);
      }
      return;
    }

    if (tecla == LogicalKeyboardKey.arrowDown) {
      if (estado.selection.isEmpty) {
        notifier.selectRow(0);
      } else {
        notifier.moveCursor(1, extend: isShift);
      }
    } else if (tecla == LogicalKeyboardKey.arrowUp) {
      if (estado.selection.cursor == 0 && !isShift) {
        _searchFocusNode.requestFocus();
      } else {
        notifier.moveCursor(-1, extend: isShift);
      }
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
      if (_showRecentSearches) {
        setState(() => _showRecentSearches = false);
        return;
      }
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


  @override
  Widget build(BuildContext context) {


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
            Stack(
              clipBehavior: Clip.none,
              children: [
                Container(
                  padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
                  decoration: const BoxDecoration(
                    color: AppColors.surface,
                    border: Border(bottom: BorderSide(color: AppColors.border)),
                  ),
                  child: Row(
                    children: [
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
                            hintText: AppStrings.isSpanish
                                ? 'Buscar en todo el equipo, unidades y discos...'
                                : 'Search across all drives and volumes...',
                            hintStyle: const TextStyle(
                                color: AppColors.textDisabled, fontSize: 13),
                            border: InputBorder.none,
                            isDense: true,
                          ),
                          onChanged: (val) {
                            ref.read(searchProvider.notifier).setQuery(val);
                          },
                          onSubmitted: (val) {
                            final q = val.trim();
                            _addRecentSearch(q);
                            setState(() => _showRecentSearches = false);
                            final pareceRuta = q.contains(Platform.pathSeparator) ||
                                (Platform.isWindows && RegExp(r'^[A-Za-z]:').hasMatch(q)) ||
                                q == '/' || q == '~' || q.startsWith('~/') || q.startsWith('~\\');
                            if (pareceRuta) {
                              _abrirComoRutaSiLoEs(q);
                            } else if (q.isNotEmpty) {
                              ref.read(searchProvider.notifier).forceSearch(q);
                            }
                          },
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
                if (_showRecentSearches)
                  Positioned(
                    top: 48,
                    left: 16,
                    right: 16,
                    child: Material(
                      elevation: 12,
                      borderRadius: BorderRadius.circular(8),
                      color: AppColors.surfaceAlt,
                      child: Container(
                        decoration: BoxDecoration(
                          borderRadius: BorderRadius.circular(8),
                          border: Border.all(color: AppColors.border),
                        ),
                        child: Column(
                          mainAxisSize: MainAxisSize.min,
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: [
                            Padding(
                              padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
                              child: Row(
                                children: [
                                  const Text(
                                    'Búsquedas recientes',
                                    style: TextStyle(
                                      fontSize: 11,
                                      fontWeight: FontWeight.w600,
                                      color: AppColors.textSecondary,
                                    ),
                                  ),
                                  const Spacer(),
                                  InkWell(
                                    onTap: _clearRecentSearches,
                                    child: const Text(
                                      'Borrar historial',
                                      style: TextStyle(fontSize: 11, color: AppColors.primary),
                                    ),
                                  ),
                                ],
                              ),
                            ),
                            const Divider(height: 1, color: AppColors.border),
                            for (final rec in _recentSearches)
                              InkWell(
                                onTap: () {
                                  _queryController.text = rec;
                                  _queryController.selection = TextSelection.collapsed(offset: rec.length);
                                  ref.read(searchProvider.notifier).setQuery(rec);
                                  _addRecentSearch(rec);
                                  setState(() => _showRecentSearches = false);
                                },
                                hoverColor: AppColors.hover,
                                child: Padding(
                                  padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 7),
                                  child: Row(
                                    children: [
                                      const Icon(Icons.history_rounded, size: 16, color: AppColors.textDisabled),
                                      const SizedBox(width: 10),
                                      Expanded(
                                        child: Text(
                                          rec,
                                          style: const TextStyle(fontSize: 13, color: AppColors.textPrimary),
                                        ),
                                      ),
                                    ],
                                  ),
                                ),
                              ),
                          ],
                        ),
                      ),
                    ),
                  ),
              ],
            ),
            Expanded(
              child: DropTarget(
                onDragDone: (detail) async {
                  if (detail.files.isEmpty) return;
                  final carpetaDestino = ref.read(searchProvider).browsePath;
                  final rutas = detail.files.map((f) => f.path).where((p) => p.isNotEmpty).toList();
                  if (rutas.isEmpty) return;

                  // Si hay una carpeta abierta en esta ventana, transferir (mover) hacia ella:
                  if (carpetaDestino.isNotEmpty && Directory(carpetaDestino).existsSync()) {
                    final sep = Platform.pathSeparator;
                    final aTransferir = rutas.where((r) =>
                        r != carpetaDestino &&
                        !carpetaDestino.startsWith('$r$sep') &&
                        !carpetaDestino.startsWith('$r/') &&
                        !carpetaDestino.startsWith('$r\\')
                    ).toList();
                    if (aTransferir.isNotEmpty) {
                      await ref.read(fileOpsProvider.notifier).moveTo(aTransferir, carpetaDestino);
                      final nombreCarpeta = carpetaDestino.split(RegExp(r'[\\/]')).lastWhere((s) => s.isNotEmpty, orElse: () => carpetaDestino);
                      _aviso(aTransferir.length == 1
                          ? 'Elemento transferido a $nombreCarpeta'
                          : '${aTransferir.length} elementos transferidos a $nombreCarpeta');
                      return;
                    }
                  }

                  // Si no hay carpeta abierta (ej. raíz de equipo), abrir el elemento soltado:
                  final path = rutas.first;
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
                            child: Row(
                              mainAxisSize: MainAxisSize.min,
                              children: [
                                const Icon(Icons.drive_file_move_rounded,
                                    color: AppColors.primary, size: 24),
                                const SizedBox(width: 12),
                                Text(
                                  ref.read(searchProvider).browsePath.isNotEmpty
                                      ? 'Soltar para transferir a ${ref.read(searchProvider).browsePath.split(RegExp(r"[\\/]")).lastWhere((s) => s.isNotEmpty, orElse: () => "esta carpeta")}'
                                      : 'Soltar para abrir',
                                  style: const TextStyle(
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
