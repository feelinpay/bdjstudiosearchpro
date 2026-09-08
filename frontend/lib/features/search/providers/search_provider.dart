import 'dart:async';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../core/ffi/api.dart' as ffi;
import '../models/engine_health.dart';
import '../models/file_row.dart';
import '../models/row_cache.dart';
import '../models/selection.dart';
import '../models/view_mode.dart';
import 'recent_folders_provider.dart';
import 'preview_provider.dart';

/// Qué está mostrando la tabla.
enum ViewMode {
  /// El resultado de una consulta.
  search,

  /// El contenido de una carpeta, leído del índice.
  browse,
}

/// Modo de agrupación de resultados en la tabla virtual.
enum GroupByMode {
  none,
  type,
  date,
}

@immutable
class SearchState {
  final ViewMode mode;
  final GroupByMode groupBy;
  final String query;
  final String activeFilter;

  /// Carpeta abierta cuando `mode` es `browse`.
  final String browsePath;

  /// Carpeta a la que se acota la búsqueda. Vacío significa todo el equipo.
  ///
  /// Es lo que hace que esto se comporte como un explorador y no como un
  /// buscador con una carpeta pegada al lado: escribir en la caja filtra **la
  /// carpeta en la que estás**, igual que en el Explorador de Windows o en el
  /// Finder, y hay un interruptor para ampliar a todo el equipo.
  final String searchScope;

  /// Cierto cuando el usuario ha pedido buscar en todo el equipo aunque esté
  /// dentro de una carpeta.
  final bool searchEverywhere;

  final List<String> history;
  final int historyIndex;

  /// 0 Nombre · 1 Ruta · 2 Extensión · 3 Tamaño · 4 Modificado · 5 Creado
  final int sortCol;
  final bool ascending;

  /// Cómo se pintan los resultados de la tabla.
  final ResultViewMode viewMode;

  final BigInt? generation;

  /// Coincidencias totales. Exacto aunque solo se hayan pedido las primeras.
  final int totalCount;

  /// Filas que el motor tiene listas para entregar.
  final int readyCount;
  final int elapsedMs;

  final Selection selection;
  final EngineHealth engine;

  final bool isCaseSensitive;
  final bool isPathMatch;
  final bool isRegex;
  final bool isSearching;

  /// Cambia cuando la caché de filas se mueve, para que la tabla se repinte.
  final int revision;

  const SearchState({
    this.mode = ViewMode.browse,
    this.groupBy = GroupByMode.none,
    this.query = '',
    this.activeFilter = 'Todos',
    this.browsePath = '',
    this.searchScope = '',
    this.searchEverywhere = true,
    this.history = const [],
    this.historyIndex = -1,
    this.sortCol = 0,
    this.ascending = true,
    this.viewMode = ResultViewMode.details,
    this.generation,
    this.totalCount = 0,
    this.readyCount = 0,
    this.elapsedMs = 0,
    this.selection = Selection.empty,
    this.engine = EngineHealth.checking,
    this.isCaseSensitive = false,
    this.isPathMatch = false,
    this.isRegex = false,
    this.isSearching = false,
    this.revision = 0,
  });

  /// Hasta dónde puede llegar la tabla.
  ///
  /// El motor entrega una ventana ordenada, no el conjunto entero: pedir la fila
  /// nueve millones exigiría ordenar nueve millones de elementos para enseñar
  /// treinta. Doscientas mil filas son muchas más de las que nadie recorre a
  /// mano, y el contador sigue diciendo el total de verdad.
  static const int maxAddressableRows = 200000;

  /// Filas direccionables. `totalCount` sigue siendo el número real de
  /// coincidencias.
  int get rowCount =>
      totalCount < maxAddressableRows ? totalCount : maxAddressableRows;

  /// Cierto cuando hay más coincidencias de las que la tabla puede recorrer.
  bool get isTruncated => totalCount > maxAddressableRows;

  BigInt get effectiveGeneration => generation ?? BigInt.zero;
  bool get isIndexLoaded => engine.isOpen;
  bool get canGoBack => historyIndex > 0;
  bool get canGoForward => historyIndex >= 0 && historyIndex < history.length - 1;

  SearchState copyWith({
    ViewMode? mode,
    GroupByMode? groupBy,
    String? query,
    String? activeFilter,
    String? browsePath,
    String? searchScope,
    bool? searchEverywhere,
    List<String>? history,
    int? historyIndex,
    int? sortCol,
    bool? ascending,
    ResultViewMode? viewMode,
    BigInt? generation,
    int? totalCount,
    int? readyCount,
    int? elapsedMs,
    Selection? selection,
    EngineHealth? engine,
    bool? isCaseSensitive,
    bool? isPathMatch,
    bool? isRegex,
    bool? isSearching,
    int? revision,
  }) {
    return SearchState(
      mode: mode ?? this.mode,
      groupBy: groupBy ?? this.groupBy,
      query: query ?? this.query,
      activeFilter: activeFilter ?? this.activeFilter,
      browsePath: browsePath ?? this.browsePath,
      searchScope: searchScope ?? this.searchScope,
      searchEverywhere: searchEverywhere ?? this.searchEverywhere,
      history: history ?? this.history,
      historyIndex: historyIndex ?? this.historyIndex,
      sortCol: sortCol ?? this.sortCol,
      ascending: ascending ?? this.ascending,
      viewMode: viewMode ?? this.viewMode,
      generation: generation ?? this.generation,
      totalCount: totalCount ?? this.totalCount,
      readyCount: readyCount ?? this.readyCount,
      elapsedMs: elapsedMs ?? this.elapsedMs,
      selection: selection ?? this.selection,
      engine: engine ?? this.engine,
      isCaseSensitive: isCaseSensitive ?? this.isCaseSensitive,
      isPathMatch: isPathMatch ?? this.isPathMatch,
      isRegex: isRegex ?? this.isRegex,
      isSearching: isSearching ?? this.isSearching,
      revision: revision ?? this.revision,
    );
  }
}

class SearchNotifier extends StateNotifier<SearchState> {
  final Ref? ref;

  SearchNotifier({this.ref, bool autoInit = true}) : super(const SearchState()) {
    if (autoInit) {
      _init();
    }
  }

  /// Cuánto se espera antes de consultar tras la última tecla.
  ///
  /// Es adaptativo según el hardware del equipo: en gama alta baja a 50 ms para
  /// una respuesta instantánea al vuelo como Everything; en gama baja sube a
  /// 160 ms para cuidar la CPU de 2 núcleos y evitar saturar el hilo principal.
  Duration _debounce = const Duration(milliseconds: 90);
  Duration get currentDebounce => _debounce;

  @visibleForTesting
  void setDebounceForTesting(Duration d) => _debounce = d;

  /// Cuánto espera la interfaz a que el motor anuncie una generación nueva.
  ///
  /// No se sondea con un temporizador: `waitIndexChanged` bloquea en el hilo
  /// nativo y solo devuelve cuando el servicio republicó (o se agota este
  /// plazo, para poder reintentar). El refresco es empujado, tipo socket.
  static const _reloadTimeout = Duration(milliseconds: 3000);

  /// Ventana que se le pide al motor de entrada.
  ///
  /// No es una constante: sale del perfil de la máquina. Pedir dos mil filas
  /// para pintar treinta es trabajo tirado justo en el equipo que menos puede
  /// permitírselo. Hasta que el motor conteste se usa un valor prudente.
  int _initialLimit = 500;

  /// Hasta dónde se amplía la ventana cuando el usuario baja de verdad.
  static const _maxLimit = SearchState.maxAddressableRows;

  Timer? _debounceTimer;
  final RowCache _cache = RowCache();

  /// Número de la última petición lanzada.
  ///
  /// Las respuestas llegan por un canal asíncrono y pueden llegar
  /// desordenadas: sin esto, el resultado de «mich» podía pisar al de
  /// «michael» y dejar en pantalla algo que el usuario ya no había pedido.
  int _seq = 0;

  Future<void> _init() async {
    await _aplicarPerfilDeMaquina();
    await _refreshEngine();
    _startWatchingIndex();
  }

  /// Ajusta la ventana, la caché y el debounce a lo que este equipo puede sostener.
  Future<void> _aplicarPerfilDeMaquina() async {
    try {
      final t = await ffi.machineTuning();
      _initialLimit = t.initialLimit;
      _cache.resize(maxPages: t.cachedPages);
      // Búsqueda en tiempo real estricta (0ms de debounce):
      // El motor Rust busca 4.5M de archivos en < 1ms con cancelación atómica.
      _debounce = Duration.zero;
      debugPrint(
        'Perfil de máquina: gama ${t.tierName}, ${t.cores} núcleos, '
        '${t.memoryMb} MB, ${t.searchThreads} hilos de búsqueda, '
        'ventana ${t.initialLimit}, ${t.cachedPages} páginas en caché, '
        'debounce ${_debounce.inMilliseconds}ms',
      );
    } catch (e) {
      // Sin perfil se sigue con los valores prudentes de arriba: es una
      // optimización, no un requisito para funcionar.
      debugPrint('No se pudo leer el perfil de la máquina: $e');
    }
  }

  // ─────────────────────────── Estado del motor ───────────────────────────

  /// Pregunta al motor cómo está y actualiza el estado.
  Future<void> _refreshEngine() async {
    try {
      await ffi.engineOpen(indexPathStr: '');
    } catch (_) {
      // El motivo concreto lo cuenta `engineStatus`; aquí no hace falta nada.
    }
    await _readEngineStatus();
    if (state.query.isNotEmpty) {
      if (state.engine.isOpen) {
        await _runSearch(state.query);
      }
    } else {
      // Arranca directamente enseñando archivos/unidades, como cualquier explorador.
      // No depende de que el índice del buscador esté abierto.
      await _runBrowse(state.browsePath, pushHistory: state.history.isEmpty);
    }
  }

  Future<void> _readEngineStatus() async {
    final esPrimeraLectura = state.engine.isChecking;
    try {
      final s = await ffi.engineStatus();
      if (!mounted) return;
      state = state.copyWith(
        engine: EngineHealth(
          isOpen: s.isOpen,
          indexPath: s.indexPath,
          fileExists: s.fileExists,
          fileSize: s.fileSize.toInt(),
          generation: s.generation.toInt(),
          entryCount: s.entryCount.toInt(),
          problem: IndexProblem.fromCode(s.problem),
          message: s.message,
          isChecking: false,
          // Solo se sella `decidedAt` en la primera lectura con veredicto. A
          // partir de ahí, los sucesivos `_readEngineStatus` no reinician el
          // periodo de gracia.
          decidedAt: esPrimeraLectura ? DateTime.now() : state.engine.decidedAt,
        ),
      );
    } catch (e) {
      if (!mounted) return;
      state = state.copyWith(
        engine: EngineHealth(
          problem: IndexProblem.unknown,
          message: 'No se pudo consultar el motor: $e',
          isChecking: false,
          decidedAt: esPrimeraLectura ? DateTime.now() : state.engine.decidedAt,
        ),
      );
    }
  }

  /// Mantiene la lista al día cuando cambian los archivos del disco.
  ///
  /// Se queda a la espera de que el motor publique una generación nueva
  /// (`waitIndexChanged`), así que el refresco llega "empujado" —como un
  /// socket— y no con sondeos periódicos desde Flutter.
  Future<void> _startWatchingIndex() async {
    while (mounted) {
      try {
        final changed = await ffi.waitIndexChanged(
          timeoutMs: BigInt.from(_reloadTimeout.inMilliseconds),
        );
        if (!mounted) return;
        if (!state.engine.isOpen) {
          await _refreshEngine();
          continue;
        }
        if (changed) {
          // Los identificadores del resultado anterior apuntan al índice viejo,
          // así que se repite la consulta entera en vez de refrescar filas.
          await _readEngineStatus();
          await _repeatCurrentView();
        }
      } catch (_) {
        // Un fallo puntual al remapear o reiniciar el servicio no debe tumbar la
        // interfaz ni girar en vacío; espera medio segundo antes de reintentar.
        await Future<void>.delayed(const Duration(milliseconds: 500));
      }
    }
  }

  StreamSubscription<FileSystemEvent>? _folderWatcher;
  Timer? _watcherDebounce;

  void _iniciarVigilanciaDirectorio(String ruta) {
    _folderWatcher?.cancel();
    _folderWatcher = null;
    try {
      final dir = Directory(ruta);
      if (dir.existsSync()) {
        _folderWatcher = dir.watch(events: FileSystemEvent.all).listen(
          (event) {
            _watcherDebounce?.cancel();
            _watcherDebounce = Timer(const Duration(milliseconds: 80), () {
              if (mounted && state.mode == ViewMode.browse && state.browsePath == ruta) {
                _repeatCurrentView();
              }
            });
          },
          onError: (e) {
            debugPrint('Error en watcher de directorio: $e');
          },
        );
      }
    } catch (_) {}
  }

  void _cancelarVigilanciaDirectorio() {
    _folderWatcher?.cancel();
    _folderWatcher = null;
    _watcherDebounce?.cancel();
    _watcherDebounce = null;
  }

  void _ordenarFilas(List<FileRow> rows, int sortCol, bool ascending) {
    rows.sort((a, b) {
      if (a.isDirectory != b.isDirectory) {
        return a.isDirectory ? -1 : 1;
      }
      int cmp = 0;
      switch (sortCol) {
        case 0:
          cmp = a.name.toLowerCase().compareTo(b.name.toLowerCase());
          break;
        case 1:
          cmp = a.extension.toLowerCase().compareTo(b.extension.toLowerCase());
          break;
        case 2:
          cmp = a.size.compareTo(b.size);
          break;
        case 3:
          cmp = a.mtime.compareTo(b.mtime);
          break;
        default:
          cmp = a.name.toLowerCase().compareTo(b.name.toLowerCase());
      }
      return ascending ? cmp : -cmp;
    });
  }

  @override
  void dispose() {
    _debounceTimer?.cancel();
    _cancelarVigilanciaDirectorio();
    super.dispose();
  }

  // ───────────────────────────── Consulta ─────────────────────────────

  /// Lo que llama la barra de búsqueda en cada tecla.
  /// Escribir en la caja **filtra la carpeta en la que estás**.
  ///
  /// Es la diferencia entre un explorador con búsqueda rápida y un buscador con
  /// una carpeta al lado. Al vaciar la caja se vuelve al contenido de la
  /// carpeta, no a una lista vacía: borrar lo que has escrito no debería
  /// dejarte en ningún sitio.
  void setQuery(String q) {
    _debounceTimer?.cancel();

    if (q.trim().isEmpty) {
      state = state.copyWith(query: '');
      _runBrowse(state.browsePath, pushHistory: false);
      return;
    }

    state = state.copyWith(
      query: q,
      mode: ViewMode.search,
      searchScope: state.searchEverywhere ? '' : (state.searchScope.isEmpty ? state.browsePath : state.searchScope),
    );
    if (_debounce == Duration.zero) {
      _runSearch(q);
    } else {
      _debounceTimer = Timer(_debounce, () => _runSearch(q));
    }
  }

  /// Alterna entre buscar en la carpeta actual y buscar en todo el equipo.
  void toggleSearchEverywhere() {
    state = state.copyWith(searchEverywhere: !state.searchEverywhere);
    if (state.query.trim().isNotEmpty) searchNow();
  }

  /// Fuerza la re-ejecución inmediata de la búsqueda (ej. al pulsar Enter en el buscador).
  /// Reactiva ViewMode.search incluso si el usuario estaba navegando por carpetas.
  void forceSearch(String q) {
    _debounceTimer?.cancel();
    if (q.trim().isEmpty) return;
    state = state.copyWith(
      query: q,
      mode: ViewMode.search,
      searchScope: state.searchEverywhere ? '' : (state.searchScope.isEmpty ? state.browsePath : state.searchScope),
    );
    _runSearch(q);
  }

  /// Lanza la búsqueda ahora mismo, sin esperar.
  Future<void> searchNow() {
    _debounceTimer?.cancel();
    return _runSearch(state.query);
  }

  /// Compatibilidad con las llamadas existentes.
  Future<void> searchFiles(String q) {
    _debounceTimer?.cancel();
    return _runSearch(q);
  }

  Future<void> _repeatCurrentView() {
    return state.mode == ViewMode.browse
        ? _runBrowse(state.browsePath, pushHistory: false)
        : _runSearch(state.query);
  }

  Future<void> _runSearch(String q) async {
    final seq = ++_seq;
    // Repetir la misma búsqueda es un refresco, no una búsqueda nueva: lo que
    // el usuario tenga marcado sigue siendo suyo.
    final mismaConsulta = state.mode == ViewMode.search && state.query == q;
    final consulta = _buildEffectiveQuery(q, state.activeFilter);
    state = state.copyWith(isSearching: true, query: q, mode: ViewMode.search);

    // Si el motor aún no está abierto (ej. indexando en segundo plano por primera vez)
    // y el usuario se encuentra dentro de una carpeta, filtrar en tiempo real dicha carpeta:
    if (!state.engine.isOpen && state.browsePath.isNotEmpty) {
      final dir = Directory(state.browsePath);
      if (dir.existsSync()) {
        final queryLower = q.toLowerCase();
        try {
          final entities = dir.listSync(followLinks: false);
          final matchingRows = <FileRow>[];
          for (final e in entities) {
            final name = e.path.split(RegExp(r'[\\/]')).last;
            if (name.toLowerCase().contains(queryLower)) {
              final isDir = e is Directory;
              try {
                final stat = e.statSync();
                final ext = isDir ? '' : (name.contains('.') ? name.split('.').last : '');
                matchingRows.add(FileRow(
                  index: matchingRows.length,
                  name: name,
                  path: state.browsePath,
                  extension: ext,
                  size: stat.size,
                  mtime: stat.modified.millisecondsSinceEpoch ~/ 1000,
                  flags: isDir ? 1 : 0,
                ));
              } catch (_) {}
            }
          }
          if (matchingRows.isNotEmpty) {
            _ordenarFilas(matchingRows, state.sortCol, state.ascending);
            for (var i = 0; i < matchingRows.length; i++) {
              final r = matchingRows[i];
              matchingRows[i] = FileRow(
                index: i,
                name: r.name,
                path: r.path,
                extension: r.extension,
                size: r.size,
                mtime: r.mtime,
                flags: r.flags,
              );
            }
            _cache.clear();
            _cache.put(0, matchingRows);
            state = state.copyWith(
              totalCount: matchingRows.length,
              readyCount: matchingRows.length,
              elapsedMs: 1,
              selection: Selection(total: matchingRows.length),
              isSearching: false,
              revision: state.revision + 1,
            );
            return;
          }
        } catch (_) {}
      }
    }

    try {
      final gen = await ffi.searchWithLimit(
        query: consulta,
        scope: _ambitoEfectivo,
        sortCol: state.sortCol,
        ascending: state.ascending,
        limit: _initialLimit,
      );
      final status = await ffi.searchStatus(generation: gen);
      if (!mounted || seq != _seq) return; // llegó tarde: ya no interesa

      _cache.clear();

      // Pre-cargar la primera página antes de notificar a la UI:
      // De esta forma la tabla pinta las filas reales en el PRIMER frame
      // sin pestañeos ni estados intermedios.
      if (status.totalCount > 0) {
        try {
          final batch = await ffi.rows(
            generation: gen,
            offset: 0,
            count: _cache.pageSize,
          );
          if (batch.count > 0) {
            final filas = <FileRow>[];
            for (var i = 0; i < batch.count; i++) {
              filas.add(FileRow(
                index: batch.offset + i,
                name: batch.names[i],
                path: batch.paths[i],
                extension: batch.extensions[i],
                size: batch.sizes[i].toInt(),
                mtime: batch.mtimes[i],
                flags: batch.flags[i],
              ));
            }
            _cache.put(0, filas);
          }
        } catch (_) {}
      }

      if (!mounted || seq != _seq) return;

      state = state.copyWith(
        generation: gen,
        totalCount: status.totalCount,
        readyCount: status.readyCount,
        elapsedMs: status.elapsedMs.toInt(),
        selection: mismaConsulta
            ? state.selection.withTotal(status.totalCount)
            : Selection(total: status.totalCount),
        isSearching: false,
        revision: state.revision + 1,
      );
    } catch (e) {
      if (!mounted || seq != _seq) return;
      _cache.clear();
      state = state.copyWith(
        totalCount: 0,
        readyCount: 0,
        elapsedMs: 0,
        selection: Selection.empty,
        isSearching: false,
        revision: state.revision + 1,
      );
      debugPrint('La búsqueda falló: $e');
    }
  }

  // ───────────────────────────── Navegación ─────────────────────────────

  /// Abre una carpeta leyendo sus hijos **del índice**, sin tocar el disco.
  Future<bool> openFolder(String path) {
    return _runBrowse(sanitizePath(path), pushHistory: true);
  }

  Future<bool> _runBrowse(String path, {required bool pushHistory}) async {
    final cleanPath = sanitizePath(path);
    final mismaCarpeta = state.mode == ViewMode.browse && state.browsePath == cleanPath;
    final seq = ++_seq;
    state = state.copyWith(isSearching: true);

    try {
      var historial = state.history;
      var indice = state.historyIndex;
      if (pushHistory) {
        // Al navegar desde un punto intermedio del historial, lo que había
        // delante se descarta: es como se comporta cualquier navegador.
        historial = [...historial.take(indice + 1), cleanPath];
        indice = historial.length - 1;
        if (cleanPath.isNotEmpty) {
          ref?.read(recentFoldersProvider.notifier).add(cleanPath);
        }
      }

      if (cleanPath.isNotEmpty) {
        final dir = Directory(cleanPath);
        if (dir.existsSync()) {
          _iniciarVigilanciaDirectorio(cleanPath);
          final entities = dir.listSync(followLinks: false);
          final rows = <FileRow>[];
          for (final e in entities) {
            final isDir = e is Directory;
            try {
              final stat = e.statSync();
              final name = e.path.split(RegExp(r'[\\/]')).last;
              if (name.isEmpty) continue;
              final ext = isDir ? '' : (name.contains('.') ? name.split('.').last : '');
              rows.add(FileRow(
                index: rows.length,
                name: name,
                path: cleanPath,
                extension: ext,
                size: stat.size,
                mtime: stat.modified.millisecondsSinceEpoch ~/ 1000,
                flags: isDir ? 1 : 0,
              ));
            } catch (_) {}
          }

          _ordenarFilas(rows, state.sortCol, state.ascending);

          for (var i = 0; i < rows.length; i++) {
            final r = rows[i];
            rows[i] = FileRow(
              index: i,
              name: r.name,
              path: r.path,
              extension: r.extension,
              size: r.size,
              mtime: r.mtime,
              flags: r.flags,
            );
          }

          _cache.clear();
          final pageSize = _cache.pageSize;
          final numPages = (rows.length / pageSize).ceil();
          for (var p = 0; p < numPages; p++) {
            final start = p * pageSize;
            final end = (start + pageSize).clamp(0, rows.length);
            _cache.put(p, rows.sublist(start, end));
          }

          state = state.copyWith(
            mode: ViewMode.browse,
            browsePath: cleanPath,
            history: historial,
            historyIndex: indice,
            generation: null,
            totalCount: rows.length,
            readyCount: rows.length,
            elapsedMs: 1,
            selection: mismaCarpeta
                ? state.selection.withTotal(rows.length)
                : Selection(total: rows.length),
            isSearching: false,
            revision: state.revision + 1,
          );
          return true;
        }
      }

      if (cleanPath.isEmpty) {
        final rows = <FileRow>[];
        // 1. Unidades de disco (C:\, D:\, etc.)
        if (Platform.isWindows) {
          for (var charCode = 65; charCode <= 90; charCode++) {
            final driveLetter = String.fromCharCode(charCode);
            final drivePath = '$driveLetter:\\';
            if (Directory(drivePath).existsSync()) {
              rows.add(FileRow(
                index: rows.length,
                name: drivePath,
                path: '',
                extension: '',
                size: 0,
                mtime: 0,
                flags: 1,
              ));
            }
          }
        } else {
          // macOS / Unix: Raíz y volúmenes USB / externos en /Volumes
          if (Directory('/').existsSync()) {
            rows.add(FileRow(
              index: rows.length,
              name: '/',
              path: '',
              extension: '',
              size: 0,
              mtime: 0,
              flags: 1,
            ));
          }
          try {
            final volDir = Directory('/Volumes');
            if (volDir.existsSync()) {
              for (final e in volDir.listSync()) {
                if (e is Directory) {
                  final name = e.path.split('/').last;
                  if (name.isNotEmpty && !name.startsWith('.')) {
                    rows.add(FileRow(
                      index: rows.length,
                      name: e.path,
                      path: '',
                      extension: '',
                      size: 0,
                      mtime: 0,
                      flags: 1,
                    ));
                  }
                }
              }
            }
          } catch (_) {}
          final home = Platform.environment['HOME'] ?? '/';
          if (home != '/' && Directory(home).existsSync()) {
            rows.add(FileRow(
              index: rows.length,
              name: home,
              path: '',
              extension: '',
              size: 0,
              mtime: 0,
              flags: 1,
            ));
          }
        }

        // 2. Carpetas recientes (mostradas al abrir el aplicativo como en el explorador)
        final recents = ref?.read(recentFoldersProvider) ?? const <String>[];
        for (final r in recents) {
          final cleanR = r.endsWith('\\') || r.endsWith('/') ? r.substring(0, r.length - 1) : r;
          if (rows.any((row) => row.fullPath == r || row.fullPath == cleanR)) continue;
          if (Directory(r).existsSync()) {
            final lastSep = cleanR.lastIndexOf(RegExp(r'[\\/]'));
            final parent = lastSep > 0 ? cleanR.substring(0, lastSep) : '';
            final name = cleanR.substring(lastSep + 1);
            rows.add(FileRow(
              index: rows.length,
              name: name,
              path: parent,
              extension: '',
              size: 0,
              mtime: 0,
              flags: 1,
            ));
          }
        }

        if (rows.isNotEmpty) {
          if (!mounted || seq != _seq) return false;
          _cache.clear();
          final pageSize = _cache.pageSize;
          final numPages = (rows.length / pageSize).ceil();
          for (var p = 0; p < numPages; p++) {
            final start = p * pageSize;
            final end = (start + pageSize).clamp(0, rows.length);
            _cache.put(p, rows.sublist(start, end));
          }
          state = state.copyWith(
            mode: ViewMode.browse,
            browsePath: '',
            history: historial,
            historyIndex: indice,
            generation: null,
            totalCount: rows.length,
            readyCount: rows.length,
            elapsedMs: 1,
            selection: Selection(total: rows.length),
            isSearching: false,
            revision: state.revision + 1,
          );
          return true;
        }
      }

      final gen = await ffi.browsePath(
        path: cleanPath,
        sortCol: state.sortCol,
        ascending: state.ascending,
        limit: _initialLimit,
      );
      final status = await ffi.searchStatus(generation: gen);
      if (!mounted || seq != _seq) return false;

      _cache.clear();
      state = state.copyWith(
        mode: ViewMode.browse,
        browsePath: cleanPath,
        history: historial,
        historyIndex: indice,
        generation: gen,
        totalCount: status.totalCount,
        readyCount: status.readyCount,
        elapsedMs: status.elapsedMs.toInt(),
        selection: mismaCarpeta
            ? state.selection.withTotal(status.totalCount)
            : Selection(total: status.totalCount),
        isSearching: false,
        revision: state.revision + 1,
      );
      await _loadPage(0);
      return true;
    } catch (e) {
      if (!mounted || seq != _seq) return false;
      state = state.copyWith(isSearching: false);
      debugPrint('No se pudo abrir la carpeta: $e');
      return false;
    }
  }

  Future<void> goBack() async {
    if (!state.canGoBack) return;
    final destino = state.history[state.historyIndex - 1];
    state = state.copyWith(historyIndex: state.historyIndex - 1);
    await _runBrowse(destino, pushHistory: false);
  }

  Future<void> goForward() async {
    if (!state.canGoForward) return;
    final destino = state.history[state.historyIndex + 1];
    state = state.copyWith(historyIndex: state.historyIndex + 1);
    await _runBrowse(destino, pushHistory: false);
  }

  /// Sube un nivel. Desde una raíz de volumen lleva a la lista de unidades.
  Future<void> goUp() async {
    if (state.mode != ViewMode.browse) return;
    final padre = await ffi.parentPath(path: state.browsePath);
    await openFolder(padre);
  }

  /// Vuelve al buscador.
  ///
  /// Con la caja vacía no hay nada que buscar, así que se queda donde está: un
  /// resultado vacío borraría de la pantalla el contenido de la carpeta sin que
  /// el usuario haya pedido nada.
  Future<void> goToSearch() async {
    if (state.query.trim().isEmpty) return;
    state = state.copyWith(mode: ViewMode.search, searchScope: state.browsePath);
    await _runSearch(state.query);
  }

  /// Abandona la búsqueda y vuelve al contenido de la carpeta.
  ///
  /// Salir de una búsqueda tiene que devolverte a donde estabas, no a una lista
  /// vacía: es la diferencia entre cerrar un filtro y perder el sitio.
  Future<void> exitSearch() async {
    state = state.copyWith(query: '', searchEverywhere: false);
    await _runBrowse(state.browsePath, pushHistory: false);
  }

  Future<List<ffi.CrumbFfi>> breadcrumbOf(String path) async {
    if (path.isEmpty) return const [];
    try {
      return await ffi.breadcrumb(path: path);
    } catch (_) {
      return const [];
    }
  }

  // ─────────────────────────────── Filas ───────────────────────────────

  /// La fila `index`, si ya está cargada. Si no, la pide y devuelve `null`.
  ///
  /// La tabla la llama solo para las filas visibles, así que en memoria hay
  /// como mucho unas dos mil, tenga el resultado tres o nueve millones.
  FileRow? rowAt(int index) {
    final fila = _cache.rowAt(index);
    if (fila != null) return fila;
    final page = _cache.pageOf(index);
    if (!_cache.isLoading(page)) {
      // Sin `await`: la tabla no puede esperar a que llegue nada.
      unawaited(_loadPage(page));
    }
    return null;
  }

  Future<void> _loadPage(int page) async {
    final gen = state.generation;
    if (gen == null) return;
    if (_cache.hasPage(page) || _cache.isLoading(page)) return;
    _cache.markLoading(page);

    final offset = page * _cache.pageSize;

    // Si la página cae más allá de lo que el motor tiene preparado, se le pide
    // una ventana mayor. Solo ocurre cuando el usuario baja de verdad.
    if (offset >= state.readyCount && state.readyCount < state.totalCount) {
      await _extendWindow(offset + _cache.pageSize * 2);
    }

    try {
      final batch = await ffi.rows(
        generation: state.effectiveGeneration,
        offset: offset,
        count: _cache.pageSize,
      );
      if (!mounted) return;
      final filas = <FileRow>[];
      for (var i = 0; i < batch.count; i++) {
        filas.add(FileRow(
          index: batch.offset + i,
          name: batch.names[i],
          path: batch.paths[i],
          extension: batch.extensions[i],
          size: batch.sizes[i].toInt(),
          mtime: batch.mtimes[i],
          flags: batch.flags[i],
        ));
      }
      _cache.put(page, filas);
      final selIdx = state.selection.firstIndex;
      if (selIdx != null && selIdx >= offset && selIdx < offset + batch.count) {
        final r = rowAt(selIdx);
        if (r != null && ref != null) {
          ref!.read(previewProvider.notifier).inspectFile(r);
        }
      }
      state = state.copyWith(revision: state.revision + 1);
    } catch (e) {
      _cache.failed(page);
      debugPrint('No se pudo cargar la página $page: $e');
    }
  }

  /// Pide al motor una ventana mayor sin cambiar la consulta.
  Future<void> _extendWindow(int hasta) async {
    final objetivo = hasta.clamp(_initialLimit, _maxLimit);
    if (objetivo <= state.readyCount) return;
    try {
      final gen = await ffi.extendResults(
        generation: state.effectiveGeneration,
        limit: objetivo,
      );
      final status = await ffi.searchStatus(generation: gen);
      if (!mounted) return;
      if (gen != state.effectiveGeneration) {
        // La ventana nueva renumera las filas: la caché anterior ya no sirve.
        _cache.clear();
      }
      state = state.copyWith(
        generation: gen,
        readyCount: status.readyCount,
        totalCount: status.totalCount,
        revision: state.revision + 1,
      );
    } catch (e) {
      debugPrint('No se pudo ampliar la ventana: $e');
    }
  }

  // ───────────────────────────── Selección ─────────────────────────────

  void selectRow(int index, {bool multi = false, bool range = false}) {
    if (index < 0 || index >= state.totalCount) return;
    final s = state.selection.withTotal(state.totalCount);
    final nueva = range
        ? s.range(index)
        : multi
            ? s.toggle(index)
            : s.single(index);
    state = state.copyWith(selection: nueva);
  }

  /// Selecciona **todo el resultado**, no solo lo que está cargado.
  void selectAll() {
    state = state.copyWith(selection: state.selection.all(state.totalCount));
  }

  void clearSelection() {
    state = state.copyWith(selection: state.selection.clear());
  }

  void invertSelection() {
    state = state.copyWith(
      selection: state.selection.withTotal(state.totalCount).invert(),
    );
  }

  void moveCursor(int delta, {bool extend = false}) {
    if (state.rowCount == 0) return;
    final destino =
        (state.selection.cursor + delta).clamp(0, state.rowCount - 1);
    final s = state.selection.withTotal(state.totalCount);
    state = state.copyWith(
      selection: extend ? s.extendCursor(destino) : s.moveCursor(destino),
    );
    // Cargar por delante para que desplazarse con el teclado no parpadee.
    rowAt(destino);
  }

  // ──────────────────────── Acciones sobre la selección ────────────────────────

  /// Rutas completas de lo seleccionado. Es lo que consume el arrastre y las operaciones de archivo.
  Future<List<String>> selectedPaths({int max = 10000}) async {
    final indices = state.selection.withTotal(state.totalCount).resolve(max: max);
    if (indices.isEmpty) return const [];

    // 1. Resolver todas las filas que ya están en la caché local (modo explorador o páginas cargadas):
    final result = <String>[];
    final pendientes = <int>[];
    for (final idx in indices) {
      final fila = _cache.rowAt(idx);
      if (fila != null && fila.fullPath.trim().isNotEmpty) {
        result.add(fila.fullPath.trim());
      } else {
        pendientes.add(idx);
      }
    }

    if (pendientes.isEmpty) {
      return result;
    }

    // 2. Si quedan filas pendientes (modo búsqueda sin cachear), pedir al motor FFI:
    final maximo = pendientes.reduce((a, b) => a > b ? a : b);
    if (maximo >= state.readyCount) {
      await _extendWindow(maximo + 1);
    }
    try {
      final rutas = await ffi.pathsForRows(
        generation: state.effectiveGeneration,
        rows: Uint32List.fromList(pendientes),
      );
      result.addAll(rutas.where((r) => r.trim().isNotEmpty));
      return result;
    } catch (e) {
      debugPrint('No se pudieron resolver las rutas: $e');
      return result;
    }
  }

  /// ¿Es la raíz de una unidad o del sistema de archivos?
  ///
  /// El motor tiene la misma comprobación y es la que manda; esta evita llegar a
  /// pedir una operación imposible y permite decírselo al usuario antes de
  /// enseñarle un diálogo de confirmación.
  static bool esRaizDeUnidad(String ruta) {
    final t = ruta.trim().replaceAll('/', r'\');
    final sinBarra =
        t.length > 1 && t.endsWith(r'\') ? t.substring(0, t.length - 1) : t;
    if (sinBarra.isEmpty || sinBarra == r'\') return true;
    if (RegExp(r'^[A-Za-z]:$').hasMatch(sinBarra)) return true;
    // Puntos de montaje: «/Volumes/Musica» es un disco entero.
    final partes = sinBarra.split(r'\').where((p) => p.isNotEmpty).toList();
    if (partes.length == 2 &&
        const ['volumes', 'mnt', 'media'].contains(partes[0].toLowerCase())) {
      return true;
    }
    return false;
  }

  /// Lo seleccionado, sin las raíces de unidad.
  ///
  /// Una unidad entera no es un archivo que se pueda mover ni borrar, y dejarla
  /// pasar terminaba en «No se pudo enviar «C:\» a la papelera».
  Future<List<String>> selectedOperablePaths({int max = 10000}) async {
    final rutas = await selectedPaths(max: max);
    return rutas.where((r) => !esRaizDeUnidad(r)).toList();
  }

  /// Ruta de una sola fila, para el arrastre de un elemento.
  Future<String> pathOf(int index) async {
    final fila = _cache.rowAt(index);
    if (fila != null && fila.fullPath.trim().isNotEmpty) {
      return fila.fullPath.trim();
    }
    try {
      return await ffi.fullPath(
        generation: state.effectiveGeneration,
        row: index,
      );
    } catch (_) {
      return '';
    }
  }

  Future<void> copySelectedPath() async {
    final rutas = await selectedPaths(max: 5000);
    if (rutas.isEmpty) return;
    await Clipboard.setData(ClipboardData(text: rutas.join('\n')));
  }

  /// Solo el nombre de archivo, sin la carpeta.
  Future<void> copySelectedName() async {
    final rutas = await selectedPaths(max: 5000);
    if (rutas.isEmpty) return;
    final nombres = rutas.map((r) {
      final i = r.lastIndexOf(RegExp(r'[\\/]'));
      return i >= 0 ? r.substring(i + 1) : r;
    });
    await Clipboard.setData(ClipboardData(text: nombres.join('\n')));
  }

  Future<void> revealSelected() async {
    final rutas = await selectedPaths(max: 1);
    if (rutas.isEmpty) return;
    try {
      await ffi.revealPath(pathStr: rutas.first);
    } catch (e) {
      debugPrint('No se pudo mostrar la ubicación: $e');
      final cambiado = await ffi.reloadIfChanged();
      if (cambiado) {
        await _readEngineStatus();
        await _repeatCurrentView();
      }
    }
  }

  /// Abre lo seleccionado. Una carpeta se abre **dentro** de la aplicación.
  Future<void> openSelected() async {
    final cursor = state.selection.cursor;
    final fila = _cache.rowAt(cursor);
    if (fila != null && fila.isDirectory) {
      await openFolder(fila.fullPath);
      return;
    }
    final rutas = await selectedPaths(max: 20);
    for (final r in rutas) {
      try {
        await ffi.openPath(pathStr: r);
      } catch (e) {
        debugPrint('No se pudo abrir «$r»: $e');
      }
    }
  }

  Future<void> openSelectedWith() async {
    final rutas = await selectedPaths(max: 1);
    if (rutas.isEmpty) return;
    try {
      await ffi.openWith(pathStr: rutas.first);
    } catch (e) {
      debugPrint('No se pudo abrir el diálogo: $e');
    }
  }

  Future<void> shareSelected() async {
    final rutas = await selectedPaths(max: 1);
    if (rutas.isEmpty) return;
    try {
      await ffi.shareFile(pathStr: rutas.first);
    } catch (e) {
      debugPrint('No se pudo abrir el diálogo de compartir: $e');
    }
  }

  Future<void> showPropertiesSelected() async {
    final rutas = await selectedPaths(max: 1);
    if (rutas.isEmpty) return;
    try {
      await ffi.showPropertiesPath(pathStr: rutas.first);
    } catch (e) {
      debugPrint('No se pudieron mostrar las propiedades: $e');
    }
  }

  // ─────────────────────────── Filtros y orden ───────────────────────────

  static const Set<String> _extensionesConocidas = {
    // Audio
    'mp3', 'wav', 'flac', 'aif', 'aiff', 'm4a', 'aac', 'ogg', 'wma', 'alac', 'mid', 'midi', 'opus',
    // Proyectos DJ
    'als', 'flp', 'cpr', 'logic', 'ptx', 'band', 'vdjcache', 'nml',
    // Vídeo
    'mp4', 'mkv', 'avi', 'mov', 'wmv', 'flv', 'webm',
    // Imágenes
    'jpg', 'jpeg', 'png', 'gif', 'webp', 'svg', 'bmp', 'ico',
    // Documentos
    'pdf', 'doc', 'docx', 'txt', 'rtf', 'xls', 'xlsx', 'csv',
    // Comprimidos
    'zip', 'rar', '7z', 'tar', 'gz', 'bz2',
    // Aplicaciones
    'exe', 'msi', 'dmg', 'pkg', 'app', 'bat', 'cmd', 'ps1', 'sh',
  };

  String _buildEffectiveQuery(String userQuery, String filter) {
    var queryStr = userQuery.trim();

    if (state.isRegex && queryStr.isNotEmpty) {
      queryStr = 'regex:"$queryStr"';
    } else if (queryStr.isNotEmpty) {
      // Detección automática de extensiones:
      // Si el usuario escribe "mp3", ".wav" o "acdc flac", se detecta la extensión automáticamente.
      final tokens = queryStr.split(RegExp(r'\s+'));
      final transformed = <String>[];
      for (final t in tokens) {
        if (t.isEmpty) continue;
        final tLower = t.toLowerCase();
        if (t.startsWith('.') && t.length > 1 && !t.contains(':')) {
          transformed.add('ext:${t.substring(1)}');
        } else if (_extensionesConocidas.contains(tLower) && !t.contains(':')) {
          transformed.add('ext:$tLower');
        } else {
          transformed.add(t);
        }
      }
      queryStr = transformed.join(' ');

      if (state.isCaseSensitive) {
        queryStr = 'case:"$queryStr"';
      }
      if (state.isPathMatch) {
        queryStr = 'ruta:"$queryStr"';
      }
    }

    final prefix = switch (filter) {
      'Audio' => 'tipo:audio ',
      'Proyectos DJ' => 'tipo:dj ',
      'Vídeo' => 'tipo:video ',
      'Imagen' => 'tipo:imagen ',
      'Documentos' => 'tipo:documentos ',
      'Comprimidos' => 'tipo:comprimidos ',
      'Aplicaciones' || 'Ejecutables' => 'tipo:apps ',
      'Carpetas' => 'tipo:carpetas ',
      _ => '',
    };
    return '$prefix$queryStr'.trim();
  }

  /// Carpeta a la que se acota la búsqueda, o cadena vacía para todo el equipo.
  ///
  /// El acotado no se mete dentro del texto de la consulta: viaja aparte hasta
  /// el motor. Metido en el texto solo podría filtrar el índice base, y las
  /// entradas de la capa de cambios —lo copiado en el último minuto— quedarían
  /// fuera precisamente cuando más interesa verlas.
  String get _ambitoEfectivo {
    if (state.searchEverywhere) return '';
    return state.searchScope;
  }

  void setFilter(String filter) {
    state = state.copyWith(activeFilter: filter);
    if (filter.isEmpty && state.query.trim().isEmpty) {
      _runBrowse(state.browsePath, pushHistory: false);
    } else {
      searchNow();
    }
  }

  void toggleCaseSensitive() {
    state = state.copyWith(isCaseSensitive: !state.isCaseSensitive);
    searchNow();
  }

  void togglePathMatch() {
    state = state.copyWith(isPathMatch: !state.isPathMatch);
    searchNow();
  }

  void toggleRegex() {
    state = state.copyWith(isRegex: !state.isRegex);
    searchNow();
  }

  void setSort(int col) {
    final asc = state.sortCol == col ? !state.ascending : true;
    state = state.copyWith(sortCol: col, ascending: asc);
    _repeatCurrentView();
  }

  /// Cambia cómo se pintan los resultados (detalles / lista / compacta).
  ///
  /// Solo afecta a la vista; no hay que volver a consultar al motor.
  void setResultView(ResultViewMode modo) {
    if (state.viewMode == modo) return;
    state = state.copyWith(viewMode: modo);
  }

  void setGroupBy(GroupByMode modo) {
    if (state.groupBy == modo) return;
    state = state.copyWith(groupBy: modo);
    if (modo == GroupByMode.type && state.sortCol != 2) {
      setSort(2);
    } else if (modo == GroupByMode.date && state.sortCol != 4) {
      setSort(4);
    }
  }

  /// Construye el CSV del resultado actual.
  ///
  /// Lee por páginas del motor en lugar de recorrer una lista en memoria: la
  /// vista ya no guarda las filas, y exportar cincuenta mil resultados no puede
  /// exigir tenerlos todos cargados a la vez.
  Future<String> buildCsv({int max = 50000}) async {
    final sb = StringBuffer();
    sb.writeln('Nombre,Ruta,Extensión,Tamaño,FechaModificación');

    final cuantas = state.totalCount < max ? state.totalCount : max;
    if (cuantas == 0) return sb.toString();

    if (cuantas > state.readyCount) {
      await _extendWindow(cuantas);
    }

    const porTanda = 500;
    for (var offset = 0; offset < cuantas; offset += porTanda) {
      final cuantasAhora =
          (cuantas - offset) < porTanda ? (cuantas - offset) : porTanda;
      final batch = await ffi.rows(
        generation: state.effectiveGeneration,
        offset: offset,
        count: cuantasAhora,
      );
      for (var i = 0; i < batch.count; i++) {
        final nombre = _escapar(batch.names[i]);
        final ruta = _escapar(batch.paths[i]);
        final ext = _escapar(batch.extensions[i]);
        final fecha = DateTime.fromMillisecondsSinceEpoch(batch.mtimes[i] * 1000)
            .toIso8601String();
        sb.writeln('"$nombre","$ruta","$ext",${batch.sizes[i]},"$fecha"');
      }
      if (batch.count == 0) break;
    }
    return sb.toString();
  }

  /// Duplica las comillas: es como se escapa una comilla dentro de un campo CSV.
  static String _escapar(String v) => v.replaceAll('"', '""');

  /// Vuelve a consultar tras una operación de archivo.
  Future<void> refreshAfterFileOperation() async {
    // La capa optimista ya publicó y recargó el overlay.bdjo al terminar fsops.
    // Comprobamos si cambió para refrescar de inmediato sin demoras perceptibles.
    var cambiado = false;
    try {
      cambiado = await ffi.reloadIfChanged();
    } catch (_) {}

    if (!cambiado) {
      // Breve margen de cortesía por si el servicio tardó unos milisegundos en publicar.
      for (var i = 0; i < 3; i++) {
        await Future<void>.delayed(const Duration(milliseconds: 50));
        try {
          if (await ffi.reloadIfChanged()) {
            break;
          }
        } catch (_) {}
      }
    }
    await _repeatCurrentView();
  }

  /// Refresco manual o por F5/«Actualizar»: recarga el índice si cambió y
  /// repite la consulta actual.
  Future<void> refreshNow() async {
    try {
      await ffi.reloadIfChanged();
    } catch (_) {}
    await _repeatCurrentView();
  }
}

final searchProvider = StateNotifierProvider<SearchNotifier, SearchState>((ref) {
  return SearchNotifier(ref: ref);
});
