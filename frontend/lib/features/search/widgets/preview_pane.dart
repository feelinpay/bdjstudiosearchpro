import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import '../../../core/theme/app_colors.dart';
import '../models/file_row.dart';
import '../providers/search_provider.dart';
import '../providers/preview_provider.dart';

/// Panel lateral derecho colapsable para previsualización de metadatos y contenido.
class PreviewPane extends ConsumerWidget {
  const PreviewPane({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final preview = ref.watch(previewProvider);
    if (!preview.isVisible) return const SizedBox.shrink();

    final row = preview.selectedRow;

    return Container(
      width: 280,
      decoration: const BoxDecoration(
        color: AppColors.surface,
        border: Border(left: BorderSide(color: AppColors.border)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          // Cabecera del panel
          Container(
            height: 34,
            padding: const EdgeInsets.symmetric(horizontal: 12),
            decoration: const BoxDecoration(
              border: Border(bottom: BorderSide(color: AppColors.border)),
            ),
            child: Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              children: [
                const Text(
                  'VISTA PREVIA',
                  style: TextStyle(
                    fontSize: 11,
                    fontWeight: FontWeight.bold,
                    color: AppColors.textSecondary,
                  ),
                ),
                IconButton(
                  icon: const Icon(Icons.close, size: 16, color: AppColors.textSecondary),
                  onPressed: () => ref.read(previewProvider.notifier).toggle(),
                  tooltip: 'Cerrar panel (Alt+P)',
                  splashRadius: 16,
                  padding: EdgeInsets.zero,
                  constraints: const BoxConstraints(minWidth: 24, minHeight: 24),
                ),
              ],
            ),
          ),

          // Contenido principal
          Expanded(
            child: row == null
                ? const Center(
                    child: Padding(
                      padding: EdgeInsets.all(24),
                      child: Text(
                        'Selecciona un elemento para ver su vista previa',
                        textAlign: TextAlign.center,
                        style: TextStyle(
                          fontSize: 12,
                          color: AppColors.textSecondary,
                        ),
                      ),
                    ),
                  )
                : preview.isLoading
                    ? const Center(child: CircularProgressIndicator(strokeWidth: 2))
                    : ListView(
                        padding: const EdgeInsets.all(16),
                        children: [
                          _buildPreviewHeader(row),
                          const SizedBox(height: 12),
                          Row(
                            children: [
                              Expanded(
                                child: ElevatedButton.icon(
                                  style: ElevatedButton.styleFrom(
                                    backgroundColor: AppColors.primary,
                                    foregroundColor: Colors.white,
                                    padding: const EdgeInsets.symmetric(vertical: 8),
                                    elevation: 0,
                                    shape: RoundedRectangleBorder(
                                      borderRadius: BorderRadius.circular(6),
                                    ),
                                  ),
                                  icon: Icon(
                                    row.isDirectory ? Icons.folder_open_rounded : Icons.play_arrow_rounded,
                                    size: 16,
                                  ),
                                  label: Text(
                                    row.isDirectory ? 'Abrir carpeta' : 'Abrir archivo',
                                    style: const TextStyle(fontSize: 12, fontWeight: FontWeight.bold),
                                  ),
                                  onPressed: () {
                                    if (row.isDirectory) {
                                      ref.read(searchProvider.notifier).openFolder(row.fullPath);
                                    } else {
                                      Process.run(Platform.isWindows ? 'explorer' : 'open', [row.fullPath]);
                                    }
                                  },
                                ),
                              ),
                            ],
                          ),
                          const SizedBox(height: 16),
                          if (preview.textContent != null) ...[
                            _buildTextPreview(preview.textContent!),
                            const SizedBox(height: 16),
                          ],
                          _buildMetadataSection(preview.metadata),
                        ],
                      ),
          ),
        ],
      ),
    );
  }

  Widget _buildPreviewHeader(FileRow row) {
    final ext = row.name.contains('.') ? row.name.split('.').last.toLowerCase() : '';
    final isImage = {'jpg', 'jpeg', 'png', 'webp', 'gif', 'bmp'}.contains(ext);
    final isAudio = {'mp3', 'wav', 'flac', 'aiff', 'aif', 'm4a', 'ogg'}.contains(ext);

    if (isImage) {
      return ClipRRect(
        borderRadius: BorderRadius.circular(6),
        child: Image.file(
          File(row.fullPath),
          height: 160,
          fit: BoxFit.cover,
          errorBuilder: (_, __, ___) => _buildFallbackIcon(row, isAudio),
        ),
      );
    }

    return _buildFallbackIcon(row, isAudio);
  }

  Widget _buildFallbackIcon(FileRow row, bool isAudio) {
    return Container(
      height: 120,
      decoration: BoxDecoration(
        color: AppColors.surfaceAlt,
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: AppColors.border),
      ),
      child: Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              row.isDirectory
                  ? Icons.folder_rounded
                  : isAudio
                      ? Icons.music_note_rounded
                      : Icons.insert_drive_file_outlined,
              size: 48,
              color: isAudio ? AppColors.primary : AppColors.textSecondary,
            ),
            const SizedBox(height: 8),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 12),
              child: Text(
                row.name,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                textAlign: TextAlign.center,
                style: const TextStyle(
                  fontSize: 12,
                  fontWeight: FontWeight.bold,
                  color: AppColors.textPrimary,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildTextPreview(String text) {
    return Container(
      constraints: const BoxConstraints(maxHeight: 200),
      padding: const EdgeInsets.all(10),
      decoration: BoxDecoration(
        color: AppColors.surfaceAlt,
        borderRadius: BorderRadius.circular(6),
        border: Border.all(color: AppColors.border),
      ),
      child: SingleChildScrollView(
        child: SelectableText(
          text,
          style: const TextStyle(
            fontSize: 11,
            fontFamily: 'monospace',
            color: AppColors.textPrimary,
          ),
        ),
      ),
    );
  }

  Widget _buildMetadataSection(Map<String, String> metadata) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const Text(
          'INFORMACIÓN',
          style: TextStyle(
            fontSize: 11,
            fontWeight: FontWeight.bold,
            color: AppColors.textSecondary,
          ),
        ),
        const SizedBox(height: 8),
        ...metadata.entries.map((entry) {
          return Padding(
            padding: const EdgeInsets.symmetric(vertical: 4),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                SizedBox(
                  width: 80,
                  child: Text(
                    entry.key,
                    style: const TextStyle(
                      fontSize: 11,
                      color: AppColors.textSecondary,
                    ),
                  ),
                ),
                Expanded(
                  child: SelectableText(
                    entry.value,
                    style: const TextStyle(
                      fontSize: 11,
                      fontWeight: FontWeight.w500,
                      color: AppColors.textPrimary,
                    ),
                  ),
                ),
              ],
            ),
          );
        }),
      ],
    );
  }
}
