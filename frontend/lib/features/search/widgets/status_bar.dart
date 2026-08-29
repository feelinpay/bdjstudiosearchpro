import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import '../../../core/theme/app_colors.dart';
import '../providers/search_provider.dart';

class SearchStatusBar extends ConsumerWidget {
  const SearchStatusBar({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final searchState = ref.watch(searchProvider);
    final countFormatted = _formatNumber(searchState.totalCount);
    final selectedRow = (searchState.visibleRows.isNotEmpty &&
            searchState.selectedIndex < searchState.visibleRows.length)
        ? searchState.visibleRows[searchState.selectedIndex]
        : null;

    return Container(
      height: 24,
      padding: const EdgeInsets.symmetric(horizontal: 16),
      decoration: const BoxDecoration(
        color: AppColors.surface,
        border: Border(top: BorderSide(color: AppColors.border)),
      ),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          // Left: Result Count & Latency
          Text(
            '$countFormatted objetos (${searchState.elapsedMs} ms)',
            style: const TextStyle(fontSize: 11, color: AppColors.textSecondary),
          ),

          // Center: Selected item details if any
          if (selectedRow != null)
            Expanded(
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 16),
                child: Text(
                  selectedRow.fullPath,
                  textAlign: TextAlign.center,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(
                    fontSize: 11,
                    color: AppColors.textSecondary,
                    fontStyle: FontStyle.italic,
                  ),
                ),
              ),
            ),

          // Right: Engine status
          Text(
            searchState.isIndexLoaded
                ? 'Índice listo · Gen ${searchState.effectiveGeneration}'
                : 'Cargando motor...',
            style: TextStyle(
              fontSize: 11,
              fontWeight: FontWeight.w500,
              color: searchState.isIndexLoaded ? AppColors.textSecondary : Colors.orange,
            ),
          ),
        ],
      ),
    );
  }

  String _formatNumber(int n) {
    return n.toString().replaceAllMapped(
          RegExp(r'(\d{1,3})(?=(\d{3})+(?!\d))'),
          (Match m) => '${m[1]}.',
        );
  }
}
