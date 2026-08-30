import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import '../../../core/ffi/api.dart' as ffi;
import '../models/file_row.dart';

class SearchState {
  final String query;
  final String activeFilter;
  final int sortCol; // 0: Name, 1: Path, 2: Ext, 3: Size, 4: Date
  final bool ascending;
  final BigInt? generation;
  final int totalCount;
  final int readyCount;
  final int elapsedMs;
  final int selectedIndex;
  final bool isIndexLoaded;
  final bool isLoadingMore;
  final List<FileRow> visibleRows;

  const SearchState({
    this.query = '',
    this.activeFilter = 'Todos',
    this.sortCol = 0,
    this.ascending = true,
    this.generation,
    this.totalCount = 0,
    this.readyCount = 0,
    this.elapsedMs = 0,
    this.selectedIndex = 0,
    this.isIndexLoaded = false,
    this.isLoadingMore = false,
    this.visibleRows = const [],
  });

  BigInt get effectiveGeneration => generation ?? BigInt.zero;
  bool get hasMore => visibleRows.length < readyCount;

  SearchState copyWith({
    String? query,
    String? activeFilter,
    int? sortCol,
    bool? ascending,
    BigInt? generation,
    int? totalCount,
    int? readyCount,
    int? elapsedMs,
    int? selectedIndex,
    bool? isIndexLoaded,
    bool? isLoadingMore,
    List<FileRow>? visibleRows,
  }) {
    return SearchState(
      query: query ?? this.query,
      activeFilter: activeFilter ?? this.activeFilter,
      sortCol: sortCol ?? this.sortCol,
      ascending: ascending ?? this.ascending,
      generation: generation ?? this.generation,
      totalCount: totalCount ?? this.totalCount,
      readyCount: readyCount ?? this.readyCount,
      elapsedMs: elapsedMs ?? this.elapsedMs,
      selectedIndex: selectedIndex ?? this.selectedIndex,
      isIndexLoaded: isIndexLoaded ?? this.isIndexLoaded,
      isLoadingMore: isLoadingMore ?? this.isLoadingMore,
      visibleRows: visibleRows ?? this.visibleRows,
    );
  }
}

class SearchNotifier extends StateNotifier<SearchState> {
  SearchNotifier() : super(const SearchState()) {
    initEngine();
  }

  /// Cada cuánto se comprueba si el servicio publicó un índice nuevo.
  ///
  /// Solo se lee la cabecera del archivo, que son microsegundos; la consulta
  /// únicamente se repite cuando la generación cambió de verdad.
  static const _reloadInterval = Duration(seconds: 1);

  Timer? _reloadTimer;

  Future<void> initEngine() async {
    try {
      await ffi.engineOpen(indexPathStr: '');
      state = state.copyWith(isIndexLoaded: true);
      await searchFiles('');
      _startWatchingIndex();
    } catch (_) {
      state = state.copyWith(isIndexLoaded: false);
    }
  }

  /// Mantiene la lista al día cuando cambian los archivos del disco.
  ///
  /// El servicio republica el índice al calmarse la actividad; sin este sondeo
  /// la aplicación seguiría mostrando el estado del arranque por muy al día que
  /// estuviera el índice en disco.
  void _startWatchingIndex() {
    _reloadTimer?.cancel();
    _reloadTimer = Timer.periodic(_reloadInterval, (_) async {
      if (!mounted) return;
      try {
        final changed = await ffi.reloadIfChanged();
        if (changed && mounted) {
          // Los identificadores del resultado anterior apuntan al índice viejo,
          // así que la consulta se repite entera en vez de refrescar filas.
          await searchFiles(state.query);
        }
      } catch (_) {
        // Un fallo puntual al remapear no debe tumbar la interfaz.
      }
    });
  }

  @override
  void dispose() {
    _reloadTimer?.cancel();
    super.dispose();
  }

  Future<void> searchFiles(String q) async {
    final effectiveQuery = _buildEffectiveQuery(q, state.activeFilter);
    try {
      final gen = await ffi.search(
        query: effectiveQuery,
        sortCol: state.sortCol,
        ascending: state.ascending,
      );

      final status = await ffi.searchStatus(generation: gen);
      final batch = await ffi.rows(generation: gen, offset: 0, count: 200);

      final rowsList = <FileRow>[];
      for (var i = 0; i < batch.count; i++) {
        rowsList.add(FileRow(
          index: batch.offset + i,
          name: batch.names[i],
          path: batch.paths[i],
          extension: batch.extensions[i],
          size: batch.sizes[i].toInt(),
          mtime: batch.mtimes[i],
          flags: batch.flags[i],
        ));
      }

      state = state.copyWith(
        query: q,
        generation: gen,
        totalCount: status.totalCount,
        readyCount: status.readyCount,
        elapsedMs: status.elapsedMs.toInt(),
        selectedIndex: 0,
        visibleRows: rowsList,
      );
    } catch (_) {
      state = state.copyWith(query: q);
    }
  }

  String _buildEffectiveQuery(String userQuery, String filter) {
    var prefix = '';
    switch (filter) {
      case 'Audio':
        prefix = 'tipo:audio ';
        break;
      case 'Proyectos DJ':
        prefix = 'tipo:dj ';
        break;
      case 'Vídeo':
        prefix = 'tipo:video ';
        break;
      case 'Imagen':
        prefix = 'tipo:imagen ';
        break;
      case 'Documentos':
        prefix = 'tipo:documentos ';
        break;
      case 'Comprimidos':
        prefix = 'tipo:comprimidos ';
        break;
      case 'Aplicaciones':
        prefix = 'tipo:apps ';
        break;
      case 'Carpetas':
        prefix = 'folder: ';
        break;
      default:
        prefix = '';
    }
    return '$prefix$userQuery'.trim();
  }

  void setFilter(String filter) {
    state = state.copyWith(activeFilter: filter);
    searchFiles(state.query);
  }

  void setSort(int col) {
    final asc = state.sortCol == col ? !state.ascending : true;
    state = state.copyWith(sortCol: col, ascending: asc);
    searchFiles(state.query);
  }

  Future<void> loadMoreRows() async {
    if (state.isLoadingMore || !state.hasMore) return;

    state = state.copyWith(isLoadingMore: true);
    try {
      final gen = state.effectiveGeneration;
      final offset = state.visibleRows.length;
      final batch = await ffi.rows(generation: gen, offset: offset, count: 200);

      final moreRows = <FileRow>[];
      for (var i = 0; i < batch.count; i++) {
        moreRows.add(FileRow(
          index: batch.offset + i,
          name: batch.names[i],
          path: batch.paths[i],
          extension: batch.extensions[i],
          size: batch.sizes[i].toInt(),
          mtime: batch.mtimes[i],
          flags: batch.flags[i],
        ));
      }

      state = state.copyWith(
        visibleRows: [...state.visibleRows, ...moreRows],
        isLoadingMore: false,
      );
    } catch (_) {
      state = state.copyWith(isLoadingMore: false);
    }
  }

  void selectRow(int index) {
    if (index >= 0 && index < state.visibleRows.length) {
      state = state.copyWith(selectedIndex: index);
    }
  }

  void selectNext() {
    if (state.selectedIndex < state.visibleRows.length - 1) {
      state = state.copyWith(selectedIndex: state.selectedIndex + 1);
      // Carga proactiva si nos acercamos al final de la ventana
      if (state.selectedIndex >= state.visibleRows.length - 30 && state.hasMore) {
        loadMoreRows();
      }
    } else if (state.hasMore) {
      loadMoreRows().then((_) {
        if (state.selectedIndex < state.visibleRows.length - 1) {
          state = state.copyWith(selectedIndex: state.selectedIndex + 1);
        }
      });
    }
  }

  void selectPrev() {
    if (state.selectedIndex > 0) {
      state = state.copyWith(selectedIndex: state.selectedIndex - 1);
    }
  }

  Future<void> copySelectedPath() async {
    if (state.visibleRows.isNotEmpty && state.selectedIndex < state.visibleRows.length) {
      final item = state.visibleRows[state.selectedIndex];
      await Clipboard.setData(ClipboardData(text: item.fullPath));
    }
  }

  Future<void> revealSelected() async {
    if (state.visibleRows.isNotEmpty && state.selectedIndex < state.visibleRows.length) {
      try {
        await ffi.revealInExplorer(
          generation: state.effectiveGeneration,
          row: state.selectedIndex,
        );
      } catch (_) {}
    }
  }
}

final searchProvider = StateNotifierProvider<SearchNotifier, SearchState>((ref) {
  return SearchNotifier();
});
