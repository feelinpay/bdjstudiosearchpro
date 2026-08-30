import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import '../../../core/theme/app_colors.dart';
import '../models/file_row.dart';
import '../providers/search_provider.dart';

class VirtualizedTable extends ConsumerStatefulWidget {
  const VirtualizedTable({super.key});

  @override
  ConsumerState<VirtualizedTable> createState() => _VirtualizedTableState();
}

class _VirtualizedTableState extends ConsumerState<VirtualizedTable> {
  final ScrollController _scrollController = ScrollController();

  @override
  void initState() {
    super.initState();
    _scrollController.addListener(_onScroll);
  }

  @override
  void dispose() {
    _scrollController.removeListener(_onScroll);
    _scrollController.dispose();
    super.dispose();
  }

  void _onScroll() {
    if (_scrollController.hasClients) {
      final maxScroll = _scrollController.position.maxScrollExtent;
      final currentScroll = _scrollController.position.pixels;
      // Disparar carga cuando queden menos de 400 px para el final
      if (maxScroll - currentScroll <= 400) {
        ref.read(searchProvider.notifier).loadMoreRows();
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final searchState = ref.watch(searchProvider);
    final notifier = ref.read(searchProvider.notifier);
    final rows = searchState.visibleRows;
    final totalItems = rows.length + (searchState.isLoadingMore ? 1 : 0);

    return Column(
      children: [
        // Column Headers
        Container(
          height: 28,
          padding: const EdgeInsets.symmetric(horizontal: 16),
          decoration: const BoxDecoration(
            color: AppColors.surface,
            border: Border(bottom: BorderSide(color: AppColors.border)),
          ),
          child: Row(
            children: [
              _buildHeaderCell('Nombre', 0, 4, searchState, notifier),
              _buildHeaderCell('Ruta', 1, 4, searchState, notifier),
              _buildHeaderCell('Ext', 2, 1, searchState, notifier),
              _buildHeaderCell('Tamaño', 3, 1, searchState, notifier, alignRight: true),
              _buildHeaderCell('Modificado', 4, 2, searchState, notifier),
            ],
          ),
        ),

        // Virtualized Rows
        Expanded(
          child: rows.isEmpty
              ? _buildEmptyState(searchState)
              : ListView.builder(
                  controller: _scrollController,
                  itemExtent: 32.0, // Fixed 32 px per row for 60 fps virtualization
                  itemCount: totalItems,
                  itemBuilder: (context, index) {
                    if (index >= rows.length) {
                      return const Center(
                        child: SizedBox(
                          height: 14,
                          width: 14,
                          child: CircularProgressIndicator(
                            strokeWidth: 2.0,
                            color: AppColors.primary,
                          ),
                        ),
                      );
                    }
                    final row = rows[index];
                    final isSelected = index == searchState.selectedIndex;
                    return _buildRow(row, index, isSelected, notifier);
                  },
                ),
        ),
      ],
    );
  }

  Widget _buildHeaderCell(
    String label,
    int colIndex,
    int flex,
    SearchState state,
    SearchNotifier notifier, {
    bool alignRight = false,
  }) {
    final isSorted = state.sortCol == colIndex;
    final arrow = isSorted ? (state.ascending ? ' ▲' : ' ▼') : '';

    return Expanded(
      flex: flex,
      child: InkWell(
        onTap: () => notifier.setSort(colIndex),
        child: Padding(
          padding: const EdgeInsets.symmetric(vertical: 4),
          child: Text(
            '$label$arrow',
            textAlign: alignRight ? TextAlign.right : TextAlign.left,
            style: TextStyle(
              fontSize: 11,
              fontWeight: isSorted ? FontWeight.w700 : FontWeight.w600,
              color: isSorted ? AppColors.primary : AppColors.textSecondary,
            ),
          ),
        ),
      ),
    );
  }

  Widget _buildRow(
    FileRow row,
    int index,
    bool isSelected,
    SearchNotifier notifier,
  ) {
    // Alternating background colors from design specs
    final bgColor = isSelected
        ? AppColors.primary
        : (index % 2 == 0 ? AppColors.rowEven : AppColors.rowOdd);

    final textColor = isSelected
        ? Colors.white
        : (row.isDisconnected ? AppColors.textDisabled : AppColors.textPrimary);

    final secondaryTextColor = isSelected
        ? Colors.white.withAlpha(200)
        : (row.isDisconnected ? AppColors.textDisabled : AppColors.textSecondary);

    return InkWell(
      onTap: () => notifier.selectRow(index),
      onDoubleTap: () => notifier.revealSelected(),
      child: Container(
        height: 32.0,
        color: bgColor,
        padding: const EdgeInsets.symmetric(horizontal: 16),
        child: Row(
          children: [
            // Name + Icon
            Expanded(
              flex: 4,
              child: Row(
                children: [
                  Icon(
                    row.isDirectory
                        ? Icons.folder_rounded
                        : _getFileIcon(row.extension),
                    size: 16,
                    color: isSelected
                        ? Colors.white
                        : (row.isDirectory ? const Color(0xFFE5A93C) : AppColors.primary),
                  ),
                  const SizedBox(width: 8),
                  Expanded(
                    child: Text(
                      row.name,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontSize: 12,
                        fontWeight: isSelected ? FontWeight.w600 : FontWeight.w400,
                        color: textColor,
                      ),
                    ),
                  ),
                ],
              ),
            ),

            // Path
            Expanded(
              flex: 4,
              child: Text(
                row.path,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(fontSize: 11, color: secondaryTextColor),
              ),
            ),

            // Ext
            Expanded(
              flex: 1,
              child: Text(
                row.extension,
                style: TextStyle(fontSize: 11, color: secondaryTextColor),
              ),
            ),

            // Size
            Expanded(
              flex: 1,
              child: Text(
                row.formattedSize,
                textAlign: TextAlign.right,
                style: TextStyle(
                  fontSize: 11,
                  fontFamily: 'monospace',
                  color: secondaryTextColor,
                ),
              ),
            ),

            // Date
            Expanded(
              flex: 2,
              child: Padding(
                padding: const EdgeInsets.only(left: 12),
                child: Text(
                  row.formattedDate,
                  style: TextStyle(fontSize: 11, color: secondaryTextColor),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  IconData _getFileIcon(String ext) {
    switch (ext.toLowerCase()) {
      case 'wav':
      case 'flac':
      case 'mp3':
      case 'aiff':
      case 'm4a':
        return Icons.music_note_rounded;
      case 'als':
      case 'flp':
      case 'ptx':
      case 'cpr':
      case 'logicx':
      case 'rpp':
        return Icons.album_rounded;
      case 'mp4':
      case 'mov':
      case 'mkv':
        return Icons.videocam_rounded;
      case 'jpg':
      case 'jpeg':
      case 'png':
      case 'webp':
        return Icons.image_rounded;
      case 'zip':
      case 'rar':
      case '7z':
        return Icons.archive_rounded;
      default:
        return Icons.insert_drive_file_outlined;
    }
  }

  Widget _buildEmptyState(SearchState state) {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(Icons.search_rounded, size: 48, color: AppColors.primary.withAlpha(80)),
          const SizedBox(height: 12),
          Text(
            state.query.isEmpty
                ? 'BDJ Studio Search Pro'
                : 'No se encontraron resultados para "${state.query}"',
            style: const TextStyle(
              fontSize: 15,
              fontWeight: FontWeight.w600,
              color: AppColors.textPrimary,
            ),
          ),
          const SizedBox(height: 4),
          Text(
            state.query.isEmpty
                ? 'Escribe en el cuadro de búsqueda para filtrar instantáneamente'
                : 'Prueba a cambiar los términos o la extensión (ej: ext:wav)',
            style: const TextStyle(fontSize: 12, color: AppColors.textSecondary),
          ),
        ],
      ),
    );
  }
}
