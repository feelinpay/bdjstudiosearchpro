import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../core/ffi/api.dart' as ffi;
import '../../search/models/file_row.dart';
import '../../search/providers/search_provider.dart';

/// Lo que hay en el portapapeles interno.
///
/// No se usa el portapapeles del sistema para las rutas porque copiar y cortar
/// tienen que distinguirse, y el portapapeles del sistema no guarda esa
/// diferencia de forma fiable entre aplicaciones.
@immutable
class ClipboardContents {
  final List<String> paths;

  /// Cierto si fue «cortar»: al pegar, se mueve en lugar de copiarse.
  final bool isCut;

  const ClipboardContents({this.paths = const [], this.isCut = false});

  bool get isEmpty => paths.isEmpty;
  bool get isNotEmpty => paths.isNotEmpty;
}

@immutable
class FileOpsState {
  /// Operaciones vivas, de la más antigua a la más reciente.
  final List<ffi.FileOpFfi> operations;
  final ClipboardContents clipboard;
  final bool canUndo;
  final bool canRedo;

  const FileOpsState({
    this.operations = const [],
    this.clipboard = const ClipboardContents(),
    this.canUndo = false,
    this.canRedo = false,
  });

  /// La primera operación detenida esperando una respuesta del usuario.
  ffi.FileOpFfi? get pendingConflict {
    for (final op in operations) {
      if (op.state == 2 && op.hasConflict) return op;
    }
    return null;
  }

  bool get hasActiveWork =>
      operations.any((op) => op.state == 0 || op.state == 1 || op.state == 2);

  FileOpsState copyWith({
    List<ffi.FileOpFfi>? operations,
    ClipboardContents? clipboard,
    bool? canUndo,
    bool? canRedo,
  }) {
    return FileOpsState(
      operations: operations ?? this.operations,
      clipboard: clipboard ?? this.clipboard,
      canUndo: canUndo ?? this.canUndo,
      canRedo: canRedo ?? this.canRedo,
    );
  }
}

/// Códigos de política ante un conflicto. Deben coincidir con `api.rs`.
class ConflictPolicy {
  static const int ask = 0;
  static const int keepBoth = 1;
  static const int skip = 2;
  static const int overwrite = 3;
}

/// Códigos de decisión ante un conflicto. Deben coincidir con `api.rs`.
class ConflictDecision {
  static const int keepBoth = 0;
  static const int skip = 1;
  static const int overwrite = 2;
  static const int cancel = 3;
}

/// Estados de una operación. Deben coincidir con `api.rs`.
class OpState {
  static const int planning = 0;
  static const int running = 1;
  static const int waitingConflict = 2;
  static const int cancelling = 3;
  static const int done = 4;
  static const int cancelled = 5;
  static const int failed = 6;
}

/// Orquesta las operaciones de archivo.
///
/// No copia ni mueve nada por su cuenta: se lo pide al gestor del motor, que
/// trabaja en sus propios hilos. Aquí solo se consulta el avance —unas pocas
/// veces por segundo— y se traduce a algo que la interfaz pueda pintar. Por eso
/// copiar cinco mil archivos no congela la ventana.
class FileOpsNotifier extends StateNotifier<FileOpsState> {
  FileOpsNotifier(this._ref) : super(const FileOpsState());

  final Ref _ref;

  /// Cada cuánto se pregunta por el avance mientras hay trabajo.
  static const _tick = Duration(milliseconds: 200);

  Timer? _timer;
  bool _pollando = false;

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  void _ensurePolling() {
    if (_timer != null) return;
    _timer = Timer.periodic(_tick, (_) => _poll());
  }

  Future<void> _poll() async {
    // refresco tras terminar puede tardar un par de segundos (hasta que el
    // indexador publica); no se apilan pasadas del mismo temporizador.
    if (_pollando) return;
    _pollando = true;
    try {
      final ops = await ffi.fsopProgressAll();
      final terminadas = await ffi.fsopReap();
      final puedeDeshacer = await ffi.fsopCanUndo();
      final puedeRehacer = await ffi.fsopCanRedo();
      if (!mounted) return;

      state = state.copyWith(
        operations: ops,
        canUndo: puedeDeshacer,
        canRedo: puedeRehacer,
      );

      if (terminadas > 0) {
        // Algo cambió en el disco: el índice ya lo sabe —se le avisó al
        // archivar— y la vista se repite para reflejarlo.
        await _ref.read(searchProvider.notifier).refreshAfterFileOperation();
      }

      if (!state.hasActiveWork) {
        _timer?.cancel();
        _timer = null;
      }
    } catch (e) {
      debugPrint('No se pudo consultar el avance: $e');
    } finally {
      _pollando = false;
    }
  }

  // ─────────────────────────── Portapapeles ───────────────────────────

  static File get _sharedClipboardFile =>
      File('${Directory.systemTemp.path}${Platform.pathSeparator}bdj_search_clipboard.json');

  Future<void> _writeSharedClipboard(List<String> paths, {required bool isCut}) async {
    try {
      _sharedClipboardFile.writeAsStringSync(jsonEncode({
        'paths': paths,
        'isCut': isCut,
        'timestamp': DateTime.now().millisecondsSinceEpoch,
      }), flush: true);
    } catch (_) {}
    try {
      await Clipboard.setData(ClipboardData(text: paths.join('\n')));
    } catch (_) {}
  }

  Future<ClipboardContents> readSharedClipboard() async {
    // 1. Portapapeles compartido en disco (entre ventanas independientes)
    try {
      if (_sharedClipboardFile.existsSync()) {
        final raw = _sharedClipboardFile.readAsStringSync();
        if (raw.trim().isNotEmpty) {
          final json = jsonDecode(raw) as Map<String, dynamic>;
          final paths = (json['paths'] as List<dynamic>?)?.map((e) => e.toString()).toList() ?? [];
          final isCut = json['isCut'] as bool? ?? false;
          final existing = paths.where((p) => File(p).existsSync() || Directory(p).existsSync()).toList();
          if (existing.isNotEmpty) {
            final res = ClipboardContents(paths: existing, isCut: isCut);
            state = state.copyWith(clipboard: res);
            return res;
          }
        }
      }
    } catch (_) {}

    // 2. Portapapeles del sistema operativo
    try {
      final data = await Clipboard.getData(Clipboard.kTextPlain);
      if (data?.text != null && data!.text!.trim().isNotEmpty) {
        final lines = data.text!.split(RegExp(r'[\r\n]+')).map((l) => l.trim()).where((l) => l.isNotEmpty).toList();
        final validPaths = lines.where((l) => File(l).existsSync() || Directory(l).existsSync()).toList();
        if (validPaths.isNotEmpty) {
          final res = ClipboardContents(paths: validPaths, isCut: false);
          state = state.copyWith(clipboard: res);
          return res;
        }
      }
    } catch (_) {}

    state = state.copyWith(clipboard: const ClipboardContents());
    return const ClipboardContents();
  }

  Future<void> copyPaths(List<String> paths, {bool isCut = false}) async {
    if (paths.isEmpty) return;
    final res = ClipboardContents(paths: paths, isCut: isCut);
    state = state.copyWith(clipboard: res);
    await _writeSharedClipboard(paths, isCut: isCut);
  }

  Future<void> copySelection() async {
    var rutas = await _ref.read(searchProvider.notifier).selectedPaths(max: 5000);
    if (rutas.isEmpty) {
      final cur = _ref.read(searchProvider).selection.cursor;
      final p = await _ref.read(searchProvider.notifier).pathOf(cur);
      if (p.isNotEmpty) rutas = [p];
    }
    if (rutas.isEmpty) return;
    await copyPaths(rutas, isCut: false);
  }

  Future<void> cutSelection() async {
    var rutas = await _ref
        .read(searchProvider.notifier)
        .selectedOperablePaths(max: 5000);
    if (rutas.isEmpty) {
      final cur = _ref.read(searchProvider).selection.cursor;
      final p = await _ref.read(searchProvider.notifier).pathOf(cur);
      if (p.isNotEmpty && !esUnidadODisco(p)) rutas = [p];
    }
    if (rutas.isEmpty) return;
    await copyPaths(rutas, isCut: true);
  }

  void clearClipboard() {
    state = state.copyWith(clipboard: const ClipboardContents());
    try {
      if (_sharedClipboardFile.existsSync()) {
        _sharedClipboardFile.deleteSync();
      }
    } catch (_) {}
  }

  /// Pega en `destino`. Devuelve el identificador de la operación, o cero.
  Future<BigInt> pasteInto(String destino) async {
    final portapapeles = await readSharedClipboard();
    if (portapapeles.isEmpty || destino.isEmpty) return BigInt.zero;

    final id = portapapeles.isCut
        ? await ffi.fsopMove(
            sources: portapapeles.paths,
            destination: destino,
            policy: ConflictPolicy.ask,
          )
        : await ffi.fsopCopy(
            sources: portapapeles.paths,
            destination: destino,
            policy: ConflictPolicy.ask,
          );

    if (portapapeles.isCut) {
      // Cortar se consume al pegar: pegarlo dos veces movería algo que ya no
      // está en su sitio.
      clearClipboard();
    }
    _ensurePolling();
    return id;
  }

  // ───────────────────────────── Operaciones ─────────────────────────────

  Future<BigInt> createFolder(String parent, String name) async {
    final id = await ffi.fsopCreateFolder(parent: parent, name: name);
    _ensurePolling();
    await _ref.read(searchProvider.notifier).refreshAfterFileOperation();
    return id;
  }

  /// Crea un archivo vacío en `parent`. Equivale a «Nuevo documento».
  Future<BigInt> createFile(String parent, String name) async {
    final id = await ffi.fsopCreateFile(parent: parent, name: name);
    _ensurePolling();
    await _ref.read(searchProvider.notifier).refreshAfterFileOperation();
    return id;
  }

  Future<BigInt> rename(String path, String newName) async {
    final id = await ffi.fsopRename(path: path, newName: newName);
    _ensurePolling();
    await _ref.read(searchProvider.notifier).refreshAfterFileOperation();
    return id;
  }

  Future<BigInt> duplicateSelection() async {
    final rutas = await _ref.read(searchProvider.notifier).selectedPaths(max: 1000);
    if (rutas.isEmpty) return BigInt.zero;
    final id = await ffi.fsopDuplicate(sources: rutas);
    _ensurePolling();
    await _ref.read(searchProvider.notifier).refreshAfterFileOperation();
    return id;
  }

  /// Envía lo seleccionado a la papelera. **Nunca borra de forma definitiva.**
  Future<BigInt> trashSelection() async {
    final rutas = await _ref
        .read(searchProvider.notifier)
        .selectedOperablePaths(max: 5000);
    if (rutas.isEmpty) return BigInt.zero;
    final id = await ffi.fsopTrash(sources: rutas);
    _ensurePolling();
    await _ref.read(searchProvider.notifier).refreshAfterFileOperation();
    return id;
  }

  /// Borra lo seleccionado de forma definitiva, sin pasar por la papelera.
  ///
  /// **No se puede deshacer.** La interfaz no debe llamar esto sin la
  /// confirmación explícita (`eliminarPermanentemente`).
  Future<BigInt> deletePermanentlySelection() async {
    final rutas = await _ref
        .read(searchProvider.notifier)
        .selectedOperablePaths(max: 5000);
    if (rutas.isEmpty) return BigInt.zero;
    final id = await ffi.fsopDeletePermanently(sources: rutas);
    _ensurePolling();
    await _ref.read(searchProvider.notifier).refreshAfterFileOperation();
    return id;
  }

  /// Restaura de la papelera a su ubicación original (rutas originales).
  Future<BigInt> restore(List<String> rutasOriginales) async {
    if (rutasOriginales.isEmpty) return BigInt.zero;
    final id = await ffi.fsopRestore(paths: rutasOriginales);
    _ensurePolling();
    return id;
  }

  Future<BigInt> moveTo(List<String> origenes, String destino) async {
    if (origenes.isEmpty || destino.isEmpty) return BigInt.zero;
    final id = await ffi.fsopMove(
      sources: origenes,
      destination: destino,
      policy: ConflictPolicy.ask,
    );
    _ensurePolling();
    return id;
  }

  /// Comprime una lista de rutas a un archivo .zip en `destinoZip`.
  Future<BigInt> compressZip(List<String> origenes, String destinoZip) async {
    if (origenes.isEmpty || destinoZip.isEmpty) return BigInt.zero;
    final id = await ffi.fsopCompressZip(
      sources: origenes,
      destination: destinoZip,
    );
    _ensurePolling();
    return id;
  }

  /// Descomprime un archivo .zip en la carpeta `destinoCarpeta`.
  Future<BigInt> extractZip(String rutaZip, String destinoCarpeta) async {
    if (rutaZip.isEmpty || destinoCarpeta.isEmpty) return BigInt.zero;
    final id = await ffi.fsopExtractZip(
      sources: [rutaZip],
      destination: destinoCarpeta,
    );
    _ensurePolling();
    return id;
  }

  Future<void> cancel(BigInt id) async {
    await ffi.fsopCancel(opId: id);
  }

  Future<void> resolveConflict(BigInt id, int decision, bool applyToAll) async {
    await ffi.fsopResolveConflict(
      opId: id,
      decision: decision,
      applyToAll: applyToAll,
    );
  }

  /// Deshace la última operación reversible.
  ///
  /// Lo que se sobrescribió no vuelve, así que esas operaciones no se ofrecen
  /// para deshacer: es preferible que el botón esté apagado a que prometa algo
  /// que no puede cumplir.
  Future<void> undoLast() async {
    final ids = await ffi.fsopUndoLast();
    if (ids.isNotEmpty) _ensurePolling();
  }

  /// Rehace la última operación que se deshizo.
  Future<void> redoLast() async {
    final ids = await ffi.fsopRedoLast();
    if (ids.isNotEmpty) _ensurePolling();
  }
}

final fileOpsProvider =
    StateNotifierProvider<FileOpsNotifier, FileOpsState>((ref) {
  return FileOpsNotifier(ref);
});
