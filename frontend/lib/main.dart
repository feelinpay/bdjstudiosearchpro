import 'dart:io';

import 'package:desktop_drop/desktop_drop.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:hotkey_manager/hotkey_manager.dart';
import 'package:tray_manager/tray_manager.dart';
import 'package:window_manager/window_manager.dart';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';

import 'core/ffi/frb_generated.dart';
import 'core/theme/app_colors.dart';
import 'core/theme/app_theme.dart';
import 'features/licensing/presentation/providers/license_providers.dart';
import 'features/licensing/presentation/screens/activation_screen.dart';
import 'features/licensing/presentation/screens/license_info_sheet.dart';
import 'features/search/providers/search_provider.dart';
import 'features/search/widgets/filter_bar.dart';
import 'features/search/widgets/status_bar.dart';
import 'features/search/widgets/virtualized_table.dart';

Future<void> _initRustLib() async {
  ExternalLibrary? externalLib;
  if (Platform.isWindows) {
    final exeDir = File(Platform.resolvedExecutable).parent.path;
    final candidates = [
      '$exeDir\\bdj_search_ffi.dll',
      '..\\engine\\target\\release\\bdj_search_ffi.dll',
      '..\\engine\\target\\debug\\bdj_search_ffi.dll',
      '..\\engine\\bdj_search_ffi\\target\\release\\bdj_search_ffi.dll',
      '..\\engine\\bdj_search_ffi\\target\\debug\\bdj_search_ffi.dll',
      'engine\\target\\release\\bdj_search_ffi.dll',
      'engine\\target\\debug\\bdj_search_ffi.dll',
    ];
    for (final c in candidates) {
      final f = File(c);
      if (f.existsSync()) {
        externalLib = ExternalLibrary.open(f.path);
        debugPrint('RustLib: cargada biblioteca nativa desde ${f.path}');
        break;
      }
    }
  } else if (Platform.isMacOS) {
    final exeDir = File(Platform.resolvedExecutable).parent.path;
    final candidates = [
      '$exeDir/libbdj_search_ffi.dylib',
      '$exeDir/../Frameworks/libbdj_search_ffi.dylib',
      '../engine/target/release/libbdj_search_ffi.dylib',
      '../engine/target/debug/libbdj_search_ffi.dylib',
      'engine/target/release/libbdj_search_ffi.dylib',
    ];
    for (final c in candidates) {
      final f = File(c);
      if (f.existsSync()) {
        externalLib = ExternalLibrary.open(f.path);
        debugPrint('RustLib: cargada biblioteca nativa desde ${f.path}');
        break;
      }
    }
  }

  try {
    if (externalLib != null) {
      await RustLib.init(externalLibrary: externalLib);
    } else {
      await RustLib.init();
    }
  } catch (e) {
    debugPrint('RustLib init error: $e');
  }
}

void _ensureIndexerRunning() {
  if (!Platform.isWindows && !Platform.isMacOS && !Platform.isLinux) return;

  try {
    final exeDir = File(Platform.resolvedExecutable).parent.path;
    final candidates = [
      '$exeDir\\bdj_search_indexer.exe',
      '..\\engine\\target\\release\\bdj_search_indexer.exe',
      '..\\engine\\target\\debug\\bdj_search_indexer.exe',
      'engine\\target\\release\\bdj_search_indexer.exe',
      'engine\\target\\debug\\bdj_search_indexer.exe',
    ];

    String? indexerExe;
    for (final c in candidates) {
      if (File(c).existsSync()) {
        indexerExe = c;
        break;
      }
    }

    if (indexerExe != null) {
      debugPrint('Asegurando indexador en segundo plano: $indexerExe');
      Process.start(indexerExe, ['--standalone'], mode: ProcessStartMode.detached);
    }
  } catch (e) {
    debugPrint('Indexer spawn note: $e');
  }
}

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await _initRustLib();
  _ensureIndexerRunning();

  if (Platform.isWindows || Platform.isMacOS || Platform.isLinux) {
    try {
      await windowManager.ensureInitialized();
      await hotKeyManager.unregisterAll();
    } catch (e) {
      debugPrint('Desktop window/hotkey manager init note: $e');
    }
  }

  runApp(const ProviderScope(child: SearchProApp()));
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

  @override
  Widget build(BuildContext context) {
    final licenseState = ref.watch(licenseProvider);

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
    with TrayListener {
  final TextEditingController _queryController = TextEditingController();
  final FocusNode _searchFocusNode = FocusNode();
  bool _isDragging = false;

  @override
  void initState() {
    super.initState();
    trayManager.addListener(this);
    _initDesktopServices();
  }

  @override
  void dispose() {
    trayManager.removeListener(this);
    _queryController.dispose();
    _searchFocusNode.dispose();
    super.dispose();
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
      exit(0);
    }
  }

  void _handleKeyEvent(KeyEvent event) {
    if (event is! KeyDownEvent) return;

    final notifier = ref.read(searchProvider.notifier);
    final isAlt = HardwareKeyboard.instance.isAltPressed;
    final isControl = HardwareKeyboard.instance.isControlPressed ||
        HardwareKeyboard.instance.isMetaPressed;

    // Alt + 1..8: filtros rápidos de categoría
    if (isAlt) {
      final filter = _quickFilters[event.logicalKey];
      if (filter != null) {
        notifier.setFilter(filter);
        return;
      }
    }

    // Ctrl / Cmd + C: copiar la ruta completa
    if (isControl && event.logicalKey == LogicalKeyboardKey.keyC) {
      notifier.copySelectedPath();
      ScaffoldMessenger.of(context)
        ..hideCurrentSnackBar()
        ..showSnackBar(
          const SnackBar(
            content: Text('Ruta copiada al portapapeles'),
            duration: Duration(milliseconds: 1200),
          ),
        );
      return;
    }

    if (event.logicalKey == LogicalKeyboardKey.arrowDown) {
      notifier.selectNext();
    } else if (event.logicalKey == LogicalKeyboardKey.arrowUp) {
      notifier.selectPrev();
    } else if (event.logicalKey == LogicalKeyboardKey.enter) {
      // Abre la ubicación del archivo seleccionado en el Explorador o el Finder.
      notifier.revealSelected();
    } else if (event.logicalKey == LogicalKeyboardKey.escape) {
      if (_queryController.text.isNotEmpty) {
        _queryController.clear();
        notifier.searchFiles('');
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    return KeyboardListener(
      focusNode: FocusNode(),
      autofocus: true,
      onKeyEvent: _handleKeyEvent,
      child: Scaffold(
        backgroundColor: AppColors.background,
        body: Column(
          children: [
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
                      decoration: const InputDecoration(
                        hintText: 'Buscar archivos instantáneamente '
                            '(ej: michael jackson ext:wav tam:>10mb)...',
                        hintStyle: TextStyle(
                            color: AppColors.textDisabled, fontSize: 13),
                        border: InputBorder.none,
                        isDense: true,
                      ),
                      onChanged: (val) {
                        ref.read(searchProvider.notifier).searchFiles(val);
                      },
                    ),
                  ),
                  if (_queryController.text.isNotEmpty)
                    IconButton(
                      icon: const Icon(Icons.clear,
                          size: 18, color: AppColors.textSecondary),
                      onPressed: () {
                        _queryController.clear();
                        ref.read(searchProvider.notifier).searchFiles('');
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
            const FilterBar(),
            Expanded(
              child: DropTarget(
                onDragDone: (detail) {
                  if (detail.files.isNotEmpty) {
                    final path = detail.files.first.path;
                    final query = 'ruta:"$path"';
                    _queryController.text = query;
                    ref.read(searchProvider.notifier).searchFiles(query);
                  }
                },
                onDragEntered: (_) => setState(() => _isDragging = true),
                onDragExited: (_) => setState(() => _isDragging = false),
                child: Stack(
                  children: [
                    const VirtualizedTable(),
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
