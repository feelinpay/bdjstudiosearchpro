import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'core/ffi/frb_generated.dart';
import 'core/theme/app_colors.dart';
import 'core/theme/app_theme.dart';
import 'features/search/providers/search_provider.dart';
import 'features/search/widgets/filter_bar.dart';
import 'features/search/widgets/status_bar.dart';
import 'features/search/widgets/virtualized_table.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  try {
    await RustLib.init();
  } catch (e) {
    debugPrint('RustLib init note: $e');
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
      home: const SearchHomeScreen(),
    );
  }
}

class SearchHomeScreen extends ConsumerStatefulWidget {
  const SearchHomeScreen({super.key});

  @override
  ConsumerState<SearchHomeScreen> createState() => _SearchHomeScreenState();
}

class _SearchHomeScreenState extends ConsumerState<SearchHomeScreen> {
  final TextEditingController _queryController = TextEditingController();
  final FocusNode _searchFocusNode = FocusNode();

  @override
  void dispose() {
    _queryController.dispose();
    _searchFocusNode.dispose();
    super.dispose();
  }

  void _handleKeyEvent(KeyEvent event) {
    if (event is! KeyDownEvent) return;

    final notifier = ref.read(searchProvider.notifier);
    final isAlt = HardwareKeyboard.instance.isAltPressed;
    final isControl = HardwareKeyboard.instance.isControlPressed ||
        HardwareKeyboard.instance.isMetaPressed;
    final isShift = HardwareKeyboard.instance.isShiftPressed;

    // Alt + 1..8: Quick Category Filter shortcuts (§9.3)
    if (isAlt) {
      if (event.logicalKey == LogicalKeyboardKey.digit1) {
        notifier.setFilter('Todos');
        return;
      } else if (event.logicalKey == LogicalKeyboardKey.digit2) {
        notifier.setFilter('Audio');
        return;
      } else if (event.logicalKey == LogicalKeyboardKey.digit3) {
        notifier.setFilter('Proyectos DJ');
        return;
      } else if (event.logicalKey == LogicalKeyboardKey.digit4) {
        notifier.setFilter('Vídeo');
        return;
      } else if (event.logicalKey == LogicalKeyboardKey.digit5) {
        notifier.setFilter('Imagen');
        return;
      } else if (event.logicalKey == LogicalKeyboardKey.digit6) {
        notifier.setFilter('Documentos');
        return;
      } else if (event.logicalKey == LogicalKeyboardKey.digit7) {
        notifier.setFilter('Comprimidos');
        return;
      } else if (event.logicalKey == LogicalKeyboardKey.digit8) {
        notifier.setFilter('Carpetas');
        return;
      }
    }

    // Ctrl + C / Cmd + C: Copy path (§11.1)
    if (isControl && event.logicalKey == LogicalKeyboardKey.keyC) {
      notifier.copySelectedPath();
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(
          content: Text('Ruta copiada al portapapeles'),
          duration: Duration(milliseconds: 1200),
        ),
      );
      return;
    }

    // Table keyboard navigation
    if (event.logicalKey == LogicalKeyboardKey.arrowDown) {
      notifier.selectNext();
    } else if (event.logicalKey == LogicalKeyboardKey.arrowUp) {
      notifier.selectPrev();
    } else if (event.logicalKey == LogicalKeyboardKey.enter) {
      if (isShift) {
        // Shift + Enter: Reveal in Explorer / Finder
        notifier.revealSelected();
      } else {
        // Enter: Open file or reveal
        notifier.revealSelected();
      }
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
            // Search Input Header
            Container(
              padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
              decoration: const BoxDecoration(
                color: AppColors.surface,
                border: Border(bottom: BorderSide(color: AppColors.border)),
              ),
              child: Row(
                children: [
                  const Icon(Icons.search_rounded, color: AppColors.primary, size: 22),
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
                        hintText:
                            'Buscar archivos instantáneamente (ej: michael jackson ext:wav tam:>10mb)...',
                        hintStyle: TextStyle(color: AppColors.textDisabled, fontSize: 13),
                        border: InputBorder.none,
                        isDense: true,
                      ),
                      onChanged: (val) {
                        // 0ms debounce: live instantaneous query evaluation (< 8 ms in Rust)
                        ref.read(searchProvider.notifier).searchFiles(val);
                      },
                    ),
                  ),
                  if (_queryController.text.isNotEmpty)
                    IconButton(
                      icon: const Icon(Icons.clear, size: 18, color: AppColors.textSecondary),
                      onPressed: () {
                        _queryController.clear();
                        ref.read(searchProvider.notifier).searchFiles('');
                      },
                    ),
                ],
              ),
            ),

            // Filter Chips Bar (Alt + 1..8)
            const FilterBar(),

            // High-density Virtualized Table
            const Expanded(
              child: VirtualizedTable(),
            ),

            // Reactive Status Bar
            const SearchStatusBar(),
          ],
        ),
      ),
    );
  }
}
