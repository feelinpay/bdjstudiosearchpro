import 'dart:io';
import 'dart:convert';
import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';
import '../models/file_row.dart';
import 'search_provider.dart';

@immutable
class PreviewState {
  final bool isVisible;
  final FileRow? selectedRow;
  final bool isLoading;
  final String? textContent;
  final Map<String, String> metadata;

  const PreviewState({
    this.isVisible = false,
    this.selectedRow,
    this.isLoading = false,
    this.textContent,
    this.metadata = const {},
  });

  PreviewState copyWith({
    bool? isVisible,
    FileRow? selectedRow,
    bool? isLoading,
    String? textContent,
    Map<String, String>? metadata,
  }) {
    return PreviewState(
      isVisible: isVisible ?? this.isVisible,
      selectedRow: selectedRow ?? this.selectedRow,
      isLoading: isLoading ?? this.isLoading,
      textContent: textContent ?? this.textContent,
      metadata: metadata ?? this.metadata,
    );
  }
}

class PreviewNotifier extends StateNotifier<PreviewState> {
  final Ref ref;
  static const _prefsKey = 'show_preview_pane';

  PreviewNotifier(this.ref) : super(const PreviewState()) {
    _loadState();
    _listenToSelection();
  }

  Future<void> _loadState() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      final visible = prefs.getBool(_prefsKey) ?? false;
      state = state.copyWith(isVisible: visible);
    } catch (_) {}
  }

  void _listenToSelection() {
    ref.listen(searchProvider, (previous, next) {
      if (!state.isVisible) return;
      final sel = next.selection;
      if (sel.isEmpty) {
        state = state.copyWith(selectedRow: null, textContent: null, metadata: {});
        return;
      }
      final firstIdx = sel.firstIndex;
      if (firstIdx != null) {
        final row = ref.read(searchProvider.notifier).rowAt(firstIdx);
        if (row != null && row.fullPath != state.selectedRow?.fullPath) {
          inspectFile(row);
        }
      }
    });
  }

  void toggle() async {
    final next = !state.isVisible;
    state = state.copyWith(isVisible: next);
    try {
      final prefs = await SharedPreferences.getInstance();
      await prefs.setBool(_prefsKey, next);
    } catch (_) {}

    if (next) {
      final search = ref.read(searchProvider);
      final firstIdx = search.selection.firstIndex;
      if (firstIdx != null) {
        final row = ref.read(searchProvider.notifier).rowAt(firstIdx);
        if (row != null) {
          inspectFile(row);
        }
      }
    }
  }

  Future<void> inspectFile(FileRow row) async {
    state = state.copyWith(selectedRow: row, isLoading: true, textContent: null, metadata: {});
    final ext = row.name.contains('.') ? row.name.split('.').last.toLowerCase() : '';

    final meta = <String, String>{
      'Nombre': row.name,
      'Ruta': row.fullPath,
      'Tamaño': row.formattedSize,
      'Tipo': row.isDirectory ? 'Carpeta' : (ext.isEmpty ? 'Archivo' : ext.toUpperCase()),
      'Modificado': row.formattedDate,
    };

    String? text;

    if (!row.isDirectory) {
      final textExts = {'txt', 'cue', 'nfo', 'log', 'json', 'xml', 'csv', 'm3u', 'm3u8'};
      if (textExts.contains(ext)) {
        try {
          final file = File(row.fullPath);
          if (await file.exists()) {
            final bytes = await file.openRead(0, 16384).toList();
            final flat = bytes.expand((x) => x).toList();
            text = utf8.decode(flat, allowMalformed: true);
          }
        } catch (_) {}
      }

      final audioExts = {'mp3', 'wav', 'flac', 'aiff', 'aif', 'm4a', 'ogg'};
      if (audioExts.contains(ext)) {
        meta['Formato'] = ext.toUpperCase();
        if (ext == 'mp3') {
          meta['Calidad'] = 'MP3 Audio';
        } else if (ext == 'wav' || ext == 'flac') {
          meta['Calidad'] = 'Lossless Audio';
        }
      }
    }

    state = state.copyWith(
      isLoading: false,
      textContent: text,
      metadata: meta,
    );
  }
}

final previewProvider = StateNotifierProvider<PreviewNotifier, PreviewState>((ref) {
  return PreviewNotifier(ref);
});
