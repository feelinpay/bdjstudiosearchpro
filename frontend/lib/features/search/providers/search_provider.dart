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
    this.searchEverywhere = false,
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
  /// Sin esto se lanzaba una búsqueda por pulsación, incluidas las que el
  /// usuario iba a sustituir cien milisegundos después. Ciento veinte
  /// milisegundos es la pausa natural entre teclas al escribir de corrido: no se
  /// nota y descarta la mayoría del trabajo inútil.
  static const _debounce = Duration(milliseconds: 120);

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

  /// Ajusta la ventana y la caché a lo que este equipo puede sostener.
  Future<void> _aplicarPerfilDeMaquina() async {
    try {
      final t = await ffi.machineTuning();
      _initialLimit = t.initialLimit;
      _cache.resize(maxPages: t.cachedPages);
      debugPrint(
        'Perfil de máquina: gama ${t.tierName}, ${t.cores} núcleos, '
        '${t.memoryMb} MB, ${t.searchThreads} hilos de búsqueda, '
        'ventana ${t.initialLimit}, ${t.cachedPages} páginas en caché',
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
    if (!state.engine.isOpen) return;
    if (state.query.isNotEmpty) {
      await _runSearch(state.query);
    } else {
      // Arranca enseñando algo, como cualquier explorador de archivos.
      //
      // Antes la primera pantalla estaba en blanco hasta que el usuario
      // escribía: eso ya de por sí no parece un explorador, parece un cuadro de
      // búsqueda. Con la ruta vacía se listan los volúmenes, que es el
      // equivalente a «Este equipo».
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

  @override
  void dispose() {
    _debounceTimer?.cancel();
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

    // El ámbito se fija al empezar a escribir y no cambia mientras se teclea:
    // así la lista no salta de carpeta a mitad de una palabra.
    final ambito = state.mode == ViewMode.browse ? state.browsePath : state.searchScope;
    state = state.copyWith(query: q, mode: ViewMode.search, searchScope: ambito);
    _debounceTimer = Timer(_debounce, () => _runSearch(q));
  }

  /// Alterna entre buscar en la carpeta actual y buscar en todo el equipo.
  void toggleSearchEverywhere() {
    state = state.copyWith(searchEverywhere: !state.searchEverywhere);
    if (state.query.trim().isNotEmpty) searchNow();
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
    final consulta = _buildEffectiveQuery(q, state.activeFilter);
    state = state.copyWith(isSearching: true, query: q, mode: ViewMode.search);

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
      state = state.copyWith(
        generation: gen,
        totalCount: status.totalCount,
        readyCount: status.readyCount,
        elapsedMs: status.elapsedMs.toInt(),
        selection: Selection(total: status.totalCount),
        isSearching: false,
        revision: state.revision + 1,
      );
      await _loadPage(0);
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
    var p = path.trim();
    if (Platform.isWindows && RegExp(r'^[a-zA-Z]:$').hasMatch(p)) {
      p = '$p\\';
    }
    return _runBrowse(p, pushHistory: true);
  }

  Future<bool> _runBrowse(String path, {required bool pushHistory}) async {
    var cleanPath = path.trim();
    if (Platform.isWindows && RegExp(r'^[a-zA-Z]:$').hasMatch(cleanPath)) {
      cleanPath = '$cleanPath\\';
    }
    final seq = ++_seq;
    state = state.copyWith(isSearching: true);

    try {
      final gen = cleanPath.isEmpty
          ? await ffi.browseRoots(sortCol: state.sortCol, ascending: state.ascending)
          : await ffi.browsePath(
              path: cleanPath,
              sortCol: state.sortCol,
              ascending: state.ascending,
              limit: _initialLimit,
            );
      final status = await ffi.searchStatus(generation: gen);
      if (!mounted || seq != _seq) return false;

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

      if (status.totalCount == 0 && cleanPath.isNotEmpty) {
        try {
          final dir = Directory(cleanPath);
          if (dir.existsSync()) {
            final entities = dir.listSync(followLinks: false);
            if (entities.isNotEmpty) {
              final fallbackRows = <FileRow>[];
              for (final e in entities) {
                final isDir = e is Directory;
                final stat = e.statSync();
                final name = e.path.split(RegExp(r'[\\/]')).last;
                if (name.isEmpty) continue;
                final ext = isDir ? '' : (name.contains('.') ? name.split('.').last : '');
                fallbackRows.add(FileRow(
                  index: fallbackRows.length,
                  name: name,
                  path: cleanPath,
                  extension: ext,
                  size: stat.size,
                  mtime: stat.modified.millisecondsSinceEpoch ~/ 1000,
                  flags: isDir ? 1 : 0,
                ));
              }
              fallbackRows.sort((a, b) {
                if (a.isDirectory != b.isDirectory) {
                  return a.isDirectory ? -1 : 1;
                }
                return a.name.toLowerCase().compareTo(b.name.toLowerCase());
              });
              for (var i = 0; i < fallbackRows.length; i++) {
                final r = fallbackRows[i];
                fallbackRows[i] = FileRow(
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
              final numPages = (fallbackRows.length / pageSize).ceil();
              for (var p = 0; p < numPages; p++) {
                final start = p * pageSize;
                final end = (start + pageSize).clamp(0, fallbackRows.length);
                _cache.put(p, fallbackRows.sublist(start, end));
              }
              state = state.copyWith(
                mode: ViewMode.browse,
                browsePath: cleanPath,
                history: historial,
                historyIndex: indice,
                generation: gen,
                totalCount: fallbackRows.length,
                readyCount: fallbackRows.length,
                elapsedMs: 1,
                selection: Selection(total: fallbackRows.length),
                isSearching: false,
                revision: state.revision + 1,
              );
              return true;
            }
          }
        } catch (e) {
          debugPrint('Error en fallback directo: $e');
        }
      }

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
        selection: Selection(total: status.totalCount),
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

  Future<List<ffi.CrumbFfi>> breadcrumbOf(String path) => ffi.breadcrumb(path: path);

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
    final row = rowAt(index);
    if (row != null && ref != null) {
      ref!.read(previewProvider.notifier).inspectFile(row);
    }
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

  /// Rutas completas de lo seleccionado. Es lo que consume el arrastre.
  Future<List<String>> selectedPaths({int max = 10000}) async {
    final indices = state.selection.withTotal(state.totalCount).resolve(max: max);
    if (indices.isEmpty) return const [];
    // Solo se pueden resolver filas dentro de la ventana que el motor tiene
    // preparada; más allá hay que ampliarla antes.
    final maximo = indices.reduce((a, b) => a > b ? a : b);
    if (maximo >= state.readyCount) {
      await _extendWindow(maximo + 1);
    }
    try {
      return await ffi.pathsForRows(
        generation: state.effectiveGeneration,
        rows: Uint32List.fromList(indices),
      );
    } catch (e) {
      debugPrint('No se pudieron resolver las rutas: $e');
      return const [];
    }
  }

  /// Ruta de una sola fila, para el arrastre de un elemento.
  Future<String> pathOf(int index) async {
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

  String _buildEffectiveQuery(String userQuery, String filter) {
    var queryStr = userQuery;

    if (state.isRegex && queryStr.isNotEmpty) {
      queryStr = 'regex:"$queryStr"';
    } else {
      if (state.isCaseSensitive && queryStr.isNotEmpty) {
        queryStr = 'case:"$queryStr"';
      }
      if (state.isPathMatch && queryStr.isNotEmpty) {
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
    searchNow();
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
