import 'dart:async';
import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:super_drag_and_drop/super_drag_and_drop.dart';

import '../../../core/i18n/app_strings.dart';
import '../../../core/theme/app_colors.dart';
import '../../fileops/dnd_utils.dart';
import '../../fileops/file_ops_dialogs.dart';
import '../../fileops/providers/file_ops_provider.dart';
import '../models/file_row.dart';
import '../models/view_mode.dart';
import '../providers/search_provider.dart';

/// La tabla de resultados.
///
/// Dos cosas la distinguen de la versión anterior:
///
/// - `itemCount` es el **total** de coincidencias, no las filas cargadas. La
///   barra de desplazamiento representa el resultado de verdad; antes mentía
///   sobre su tamaño y crecía a saltos cada vez que llegaba otra tanda.
/// - Las filas se piden por página y se olvidan al alejarse. Bajar hasta el
///   resultado un millón ya no retiene un millón de objetos en memoria.
class VirtualizedTable extends ConsumerStatefulWidget {
  const VirtualizedTable({super.key});

  static BuildContext? _activeMenuContext;
  static final String _sharedMenuFile = '${Directory.systemTemp.path}/bdj_active_menu.json';
  static int _myLastMenuOpenedTime = 0;
  static Timer? _menuWatchTimer;

  /// Cierra cualquier menú contextual abierto en esta ventana.
  static void dismissActiveMenu() {
    final ctx = _activeMenuContext;
    if (ctx != null && ctx.mounted) {
      Navigator.of(ctx, rootNavigator: true).maybePop();
    }
    _activeMenuContext = null;
    _menuWatchTimer?.cancel();
    _menuWatchTimer = null;
  }

  static void _notifyMenuOpened(BuildContext context) {
    dismissActiveMenu();
    _activeMenuContext = context;
    final now = DateTime.now().millisecondsSinceEpoch;
    _myLastMenuOpenedTime = now;
    try {
      File(_sharedMenuFile).writeAsStringSync(now.toString());
    } catch (_) {}

    _menuWatchTimer?.cancel();
    _menuWatchTimer = Timer.periodic(const Duration(milliseconds: 150), (timer) {
      if (_activeMenuContext == null) {
        timer.cancel();
        return;
      }
      try {
        final f = File(_sharedMenuFile);
        if (f.existsSync()) {
          final content = f.readAsStringSync().trim();
          final otherTime = int.tryParse(content) ?? 0;
          if (otherTime > _myLastMenuOpenedTime) {
            // Otra ventana abrió un menú: cerramos el nuestro de inmediato
            dismissActiveMenu();
            timer.cancel();
          }
        }
      } catch (_) {}
    });
  }

  static void _clearMenuOpened() {
    _activeMenuContext = null;
    _menuWatchTimer?.cancel();
    _menuWatchTimer = null;
  }

  @override
  ConsumerState<VirtualizedTable> createState() => _VirtualizedTableState();
}

class _VirtualizedTableState extends ConsumerState<VirtualizedTable> {
  final ScrollController _scrollController = ScrollController();

  static const double _rowHeight = 32.0;
  static const double _rowHeightCompacta = 24.0;

  double _altoFila(ResultViewMode modo) =>
      modo == ResultViewMode.compact ? _rowHeightCompacta : _rowHeight;

  @override
  void dispose() {
    _scrollController.dispose();
    super.dispose();
  }

  void _asegurarVisible(int index, ResultViewMode viewMode) {
    if (!mounted || !_scrollController.hasClients || index < 0) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted || !_scrollController.hasClients || index < 0) return;
        _hacerScrollVisible(index, viewMode);
      });
      return;
    }
    _hacerScrollVisible(index, viewMode);
  }

  void _hacerScrollVisible(int index, ResultViewMode viewMode) {
    if (!_scrollController.hasClients) return;
    final alto = viewMode == ResultViewMode.grid ? 120.0 : _altoFila(viewMode);
    final targetTop = index * alto;
    final targetBottom = targetTop + alto;
    final currentOffset = _scrollController.offset;
    final viewportHeight = _scrollController.position.viewportDimension;

    if (targetTop < currentOffset) {
      _scrollController.jumpTo(targetTop);
    } else if (targetBottom > currentOffset + viewportHeight) {
      _scrollController.jumpTo(
        (targetBottom - viewportHeight).clamp(0.0, _scrollController.position.maxScrollExtent),
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    ref.listen(searchProvider, (anterior, actual) {
      if (anterior?.selection.cursor != actual.selection.cursor) {
        _asegurarVisible(actual.selection.cursor, actual.viewMode);
      }
    });

    final estado = ref.watch(searchProvider);
    final notifier = ref.read(searchProvider.notifier);
    final clipboard = ref.watch(fileOpsProvider.select((s) => s.clipboard));
    final viewMode = estado.viewMode;
    final esGrid = viewMode == ResultViewMode.grid;

    final cuerpo = estado.rowCount == 0
        ? _EstadoVacio(estado: estado)
        : esGrid
            ? _grid(estado, notifier, clipboard)
            : ListView.builder(
                controller: _scrollController,
                itemExtent: estado.groupBy == GroupByMode.none ? _altoFila(viewMode) : null,
                itemCount: estado.rowCount,
                itemBuilder: (context, index) {
                  final fila = notifier.rowAt(index);
                  if (fila == null) {
                    return _FilaPendiente(
                      index: index,
                      alto: _altoFila(viewMode),
                    );
                  }
                  final item = _Fila(
                    fila: fila,
                    index: index,
                    viewMode: viewMode,
                    mostrarRuta: estado.mode == ViewMode.search,
                    seleccionada: estado.selection.contains(index),
                    cortada: clipboard.isCut && clipboard.paths.contains(fila.fullPath),
                    onTapDown: () => _onRowTapDown(index),
                    onContextMenu: (posicion) => _menuContextual(
                        context, notifier, ref.read(searchProvider), index, posicion),
                    rutasArrastradas: () => _rutasParaArrastrar(index),
                    onSoltarEnCarpeta: (rutas, destino) =>
                        _moverSoltados(rutas, destino),
                  );

                  if (estado.groupBy == GroupByMode.none) {
                    return item;
                  }

                  final grupoActual = _grupoDeFila(fila, estado.groupBy);
                  bool esInicio = false;
                  if (index == 0) {
                    esInicio = true;
                  } else {
                    final anterior = notifier.rowAt(index - 1);
                    if (anterior != null && _grupoDeFila(anterior, estado.groupBy) != grupoActual) {
                      esInicio = true;
                    }
                  }

                  if (esInicio && grupoActual.isNotEmpty) {
                    return Column(
                      mainAxisSize: MainAxisSize.min,
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        _CabeceraGrupo(titulo: grupoActual),
                        item,
                      ],
                    );
                  }

                  return item;
                },
              );

    final destinoCarpeta = estado.mode == ViewMode.browse ? estado.browsePath : '';
    Widget dropArea = GestureDetector(
      behavior: HitTestBehavior.translucent,
      onTapDown: (_) => notifier.clearSelection(),
      onSecondaryTapDown: (d) => _menuDeFondo(context, d.globalPosition),
      child: cuerpo,
    );

    if (destinoCarpeta.isNotEmpty) {
      dropArea = DropRegion(
        formats: kFileDropFormats,
        hitTestBehavior: HitTestBehavior.translucent,
        onDropOver: (event) async {
          if (dropSessionTieneArchivos(event.session)) {
            return DropOperation.copy;
          }
          return DropOperation.none;
        },
        onPerformDrop: (event) async {
          final rutas = await extraerRutasDeDropSession(event.session);
          if (rutas.isNotEmpty) {
            _moverSoltados(rutas, destinoCarpeta);
          }
        },
        child: dropArea,
      );
    }

    return Column(
      children: [
        if (!esGrid)
          _Cabecera(estado: estado, notifier: notifier, viewMode: viewMode),
        Expanded(
          child: dropArea,
        ),
      ],
    );
  }

  /// Vista de iconos: cuadrícula virtualizada de tarjetas.
  ///
  /// Cada tarjeta es un icono grande con el nombre debajo. Acepta lo mismo que
  /// una fila —selección, doble clic, menú, arrastre al sistema y sueltas sobre
  /// carpetas— porque un explorador no debe cambiar de personalidad según la
  /// vista.
  Widget _grid(SearchState estado, SearchNotifier notifier, ClipboardContents clipboard) {
    return GridView.builder(
      controller: _scrollController,
      padding: const EdgeInsets.all(12),
      gridDelegate: const SliverGridDelegateWithMaxCrossAxisExtent(
        maxCrossAxisExtent: 120,
        mainAxisSpacing: 10,
        crossAxisSpacing: 10,
        childAspectRatio: 0.75,
      ),
      itemCount: estado.rowCount,
      itemBuilder: (context, index) {
        final fila = notifier.rowAt(index);
        if (fila == null) return const _CasillaPendiente();
        return _CasillaIcono(
          fila: fila,
          seleccionada: estado.selection.contains(index),
          cortada: clipboard.isCut && clipboard.paths.contains(fila.fullPath),
          onTapDown: () => _onRowTapDown(index),
          onContextMenu: (posicion) => _menuContextual(
              context, notifier, ref.read(searchProvider), index, posicion),
          rutasArrastradas: () => _rutasParaArrastrar(index),
          onSoltarEnCarpeta: (rutas, destino) => _moverSoltados(rutas, destino),
        );
      },
    );
  }

  int _lastClickTime = 0;
  int _lastClickIndex = -1;

  void _onRowTapDown(int index) {
    final now = DateTime.now().millisecondsSinceEpoch;
    // 0 milisegundos de latencia: respuesta táctil inmediata sin esperar al timeout del gesture arena
    _alPulsar(index);

    if (_lastClickIndex == index && (now - _lastClickTime) < 350) {
      _lastClickTime = 0;
      _lastClickIndex = -1;
      ref.read(searchProvider.notifier).openSelected();
    } else {
      _lastClickTime = now;
      _lastClickIndex = index;
    }
  }

  void _alPulsar(int index) {
    final notifier = ref.read(searchProvider.notifier);
    final teclado = HardwareKeyboard.instance;
    final conCtrl = teclado.isControlPressed || teclado.isMetaPressed;
    final conMayus = teclado.isShiftPressed;
    notifier.selectRow(index, multi: conCtrl, range: conMayus);
  }

  /// Las rutas que se entregan al sistema al arrastrar.
  ///
  /// Si la fila pulsada forma parte de la selección se arrastra **toda** la
  /// selección; si no, solo ella. Es lo que hace cualquier explorador, y es lo
  /// que permite marcar cuatro pistas y soltarlas de golpe en el DAW.
  Future<List<String>> _rutasParaArrastrar(int index) async {
    final estado = ref.read(searchProvider);
    final notifier = ref.read(searchProvider.notifier);
    List<String> raw;
    if (estado.selection.contains(index) && estado.selection.count > 1) {
      raw = await notifier.selectedPaths(max: 5000);
    } else {
      final ruta = await notifier.pathOf(index);
      raw = ruta.isEmpty ? const [] : [ruta];
    }
    return raw.where((r) => !esUnidadODisco(r)).toList();
  }

  /// Mueve lo soltado dentro de una carpeta.
  Future<void> _moverSoltados(List<String> rutas, String destino) async {
    if (rutas.isEmpty || destino.isEmpty) return;
    if (rutas.contains(destino)) return;
    final filtradas = rutas.where((r) => !esUnidadODisco(r)).toList();
    if (filtradas.isEmpty) return;

    final ops = ref.read(fileOpsProvider.notifier);
    await ops.moveTo(filtradas, destino);
    await ref.read(searchProvider.notifier).refreshAfterFileOperation();
  }

  Future<void> _menuContextual(
    BuildContext context,
    SearchNotifier notifier,
    SearchState estado,
    int index,
    Offset posicion,
  ) async {
    if (!estado.selection.contains(index)) {
      notifier.selectRow(index);
      // Estado fresco: seleccionar una fila nueva puede haber cambiado el
      // conteo con el que se decide qué opciones se ofrecen.
      estado = ref.read(searchProvider);
    }
    final varios = estado.selection.count > 1;
    final ops = ref.read(fileOpsProvider.notifier);
    final clipboard = await ops.readSharedClipboard();
    if (!context.mounted) return;
    final filaClickeada = notifier.rowAt(index);
    final esCarpetaClickeada = filaClickeada?.isDirectory ?? false;
    final destinoPegar = esCarpetaClickeada
        ? (filaClickeada?.fullPath ?? estado.browsePath)
        : estado.browsePath;

    VirtualizedTable._notifyMenuOpened(context);
    final valor = await showMenu<String>(
      context: context,
      position: RelativeRect.fromLTRB(posicion.dx, posicion.dy, posicion.dx, posicion.dy),
      elevation: 4,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(4)),
      items: <PopupMenuEntry<String>>[
        _item('refresh', AppStrings.refresh),
        _item('open', varios ? (AppStrings.isSpanish ? 'Abrir los seleccionados' : 'Open selected') : AppStrings.open),
        const PopupMenuDivider(),
        _item('cut', varios ? (AppStrings.isSpanish ? 'Cortar los seleccionados' : 'Cut selected') : AppStrings.cut),
        _item('copy', varios ? (AppStrings.isSpanish ? 'Copiar los seleccionados' : 'Copy selected') : AppStrings.copy),
        _item('paste', AppStrings.paste, habilitado: clipboard.isNotEmpty && destinoPegar.isNotEmpty),
        _item('duplicate', AppStrings.duplicate),
        if (!varios) _item('rename', AppStrings.rename),
        const PopupMenuDivider(),
        _item('zip', AppStrings.compressZip),
        if (notifier.rowAt(index)?.fullPath.toLowerCase().endsWith('.zip') == true)
          _item('unzip', AppStrings.decompressHere),
        _item('reveal', AppStrings.revealLocation),
        _item('copypath', varios ? AppStrings.copyPaths : AppStrings.copyPath),
        _item('copyname', varios ? AppStrings.copyNames : AppStrings.copyName),
        const PopupMenuDivider(),
        _item('selectall', AppStrings.selectAll),
        _item('invert', AppStrings.invertSelection),
        const PopupMenuDivider(),
        _item('trash', AppStrings.delete),
        _item('deleteforever', AppStrings.deleteForever),
      ],
    );
    VirtualizedTable._clearMenuOpened();
    if (!context.mounted) return;

    switch (valor) {
      case 'refresh':
        notifier.refreshNow();
      case 'open':
        notifier.openSelected();
      case 'reveal':
        notifier.revealSelected();
      case 'copypath':
        notifier.copySelectedPath();
      case 'copyname':
        notifier.copySelectedName();
      case 'cut':
        var rutas = await notifier.selectedPaths(max: 5000);
        if (rutas.isEmpty) {
          final p = notifier.rowAt(index)?.fullPath ?? await notifier.pathOf(index);
          if (p.isNotEmpty && !esUnidadODisco(p)) rutas = [p];
        }
        await ops.copyPaths(rutas, isCut: true);
      case 'copy':
        var rutas = await notifier.selectedPaths(max: 5000);
        if (rutas.isEmpty) {
          final p = notifier.rowAt(index)?.fullPath ?? await notifier.pathOf(index);
          if (p.isNotEmpty) rutas = [p];
        }
        await ops.copyPaths(rutas, isCut: false);
      case 'paste':
        if (destinoPegar.isNotEmpty) {
          await ops.pasteInto(destinoPegar);
        }
      case 'duplicate':
        await ops.duplicateSelection();
      case 'rename':
        await renombrarDialog(context, ref, ops);
      case 'selectall':
        notifier.selectAll();
      case 'invert':
        notifier.invertSelection();
      case 'trash':
        await confirmarEnviarALaPapelera(context, ops, estado.selection.count);
      case 'deleteforever':
        await eliminarPermanentemente(context, ops, estado.selection.count);
      case 'zip':
        final rutas = await notifier.selectedPaths(max: 5000);
        if (rutas.isNotEmpty) {
          final primero = rutas.first;
          final baseDir = primero.contains(RegExp(r'[\\/]'))
              ? primero.substring(0, primero.lastIndexOf(RegExp(r'[\\/]')))
              : '';
          final sep = primero.contains('\\') ? '\\' : '/';
          final nombreBase = primero.split(RegExp(r'[\\/]')).last;
          final sinExt = nombreBase.contains('.')
              ? nombreBase.substring(0, nombreBase.lastIndexOf('.'))
              : nombreBase;
          final nombreZip = rutas.length == 1
              ? '$sinExt.zip'
              : 'archivo_comprimido.zip';
          final destino = baseDir.isEmpty ? nombreZip : '$baseDir$sep$nombreZip';
          await ops.compressZip(rutas, destino);
        }
      case 'unzip':
        final rutas = await notifier.selectedPaths(max: 10);
        for (final r in rutas) {
          if (r.toLowerCase().endsWith('.zip')) {
            final carpetaDestino = r.substring(0, r.length - 4);
            await ops.extractZip(r, carpetaDestino);
          }
        }
    }
  }

  /// Menú del fondo: lo que aparece al pulsar con el botón derecho donde no
  /// hay ningún elemento. Crea carpetas y archivos, pega lo que esté en el
  /// portapapeles y selecciona, como el Explorador.
  Future<void> _menuDeFondo(BuildContext context, Offset posicion) async {
    final estado = ref.read(searchProvider);
    final ops = ref.read(fileOpsProvider.notifier);
    final clipboard = await ops.readSharedClipboard();
    if (!context.mounted) return;
    final destino = estado.browsePath;
    final enCarpeta = estado.mode == ViewMode.browse && destino.isNotEmpty;

    VirtualizedTable._notifyMenuOpened(context);
    final valor = await showMenu<String>(
      context: context,
      position: RelativeRect.fromLTRB(posicion.dx, posicion.dy, posicion.dx, posicion.dy),
      elevation: 4,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(4)),
      items: <PopupMenuEntry<String>>[
        if (enCarpeta) ...[
          _item('newfolder', AppStrings.newFolder),
          _item('newfile', AppStrings.newFile),
          _item('paste', AppStrings.paste, habilitado: clipboard.isNotEmpty),
          const PopupMenuDivider(),
        ],
        _item('selectall', AppStrings.selectAll),
        _item('invert', AppStrings.invertSelection),
      ],
    );
    VirtualizedTable._clearMenuOpened();
    if (!context.mounted) return;

    switch (valor) {
      case 'newfolder':
        await nuevaCarpetaDialog(context, ops, destino);
      case 'newfile':
        await nuevoArchivoDialog(context, ops, destino);
      case 'paste':
        await ops.pasteInto(destino);
      case 'selectall':
        ref.read(searchProvider.notifier).selectAll();
      case 'invert':
        ref.read(searchProvider.notifier).invertSelection();
    }
  }

  PopupMenuItem<String> _item(String valor, String texto, {bool habilitado = true}) =>
      PopupMenuItem<String>(
        value: valor,
        enabled: habilitado,
        height: 32,
        child: Text(texto, style: const TextStyle(fontSize: 13)),
      );
}

/// Icono por extensión, compartido por las filas y las tarjetas de iconos.
IconData iconoExtension(String ext) {
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

/// Envuelve una fila o tarjeta para que se pueda arrastrar al sistema y, si es
/// una carpeta, acepte sueltas internas (mover/copiar a esa carpeta).
Widget envolverConArrastre({
  required Widget child,
  required FileRow fila,
  required Future<List<String>> Function() rutasArrastradas,
  void Function(List<String> rutas, String destino)? onSoltarEnCarpeta,
}) {
  final esDisco = esUnidadODisco(fila.fullPath);

  final base = esDisco
      ? child
      : DragItemWidget(
          allowedOperations: () => [DropOperation.copy, DropOperation.move],
          dragItemProvider: (request) async {
            final rutas = await rutasArrastradas();
            if (rutas.isEmpty) return null;
            final item = DragItem(localData: rutas.join('\n'));
            for (final r in rutas) {
              item.add(Formats.fileUri(Uri.file(r)));
            }
            item.add(Formats.plainText(rutas.join('\n')));
            return item;
          },
          child: DraggableWidget(
            onDragConfiguration: (config, session) async {
              final rutas = await rutasArrastradas();
              if (rutas.isEmpty) return null;
              final snapshot = config.items.isNotEmpty ? config.items.first.image : null;
              if (snapshot == null) return config;

              final items = <DragConfigurationItem>[];
              for (final r in rutas) {
                final item = DragItem(localData: r);
                item.add(Formats.fileUri(Uri.file(r)));
                item.add(Formats.plainText(r));
                items.add(DragConfigurationItem(item: item, image: snapshot));
              }
              return DragConfiguration(
                items: items,
                allowedOperations: config.allowedOperations,
              );
            },
            child: child,
          ),
        );

  final soltar = onSoltarEnCarpeta;
  if (fila.isDirectory && soltar != null) {
    return DropRegion(
      formats: kFileDropFormats,
      onDropOver: (event) async {
        if (dropSessionTieneArchivos(event.session)) {
          return DropOperation.copy;
        }
        return DropOperation.none;
      },
      onPerformDrop: (event) async {
        final rutas = await extraerRutasDeDropSession(event.session);
        if (rutas.isNotEmpty) {
          soltar(rutas, fila.fullPath);
        }
      },
      child: base,
    );
  }
  return base;
}

class _Cabecera extends StatelessWidget {
  const _Cabecera({
    required this.estado,
    required this.notifier,
    required this.viewMode,
  });

  final SearchState estado;
  final SearchNotifier notifier;
  final ResultViewMode viewMode;

  @override
  Widget build(BuildContext context) {
    // En vista compacta no hay cabecera: cada fila es la mínima expresión.
    if (viewMode == ResultViewMode.compact) {
      return const SizedBox.shrink();
    }

    final lista = viewMode == ResultViewMode.list;
    final mostrarRuta = estado.mode == ViewMode.search;
    final flexNombre = lista ? (mostrarRuta ? 5 : 9) : (mostrarRuta ? 5 : 8);
    return Container(
      height: 28,
      padding: const EdgeInsets.symmetric(horizontal: 16),
      decoration: const BoxDecoration(
        color: AppColors.surface,
        border: Border(bottom: BorderSide(color: AppColors.border)),
      ),
      child: Row(
        children: [
          _celda('Nombre', 0, flexNombre, padding: const EdgeInsets.only(right: 8, top: 4, bottom: 4)),
          if (mostrarRuta)
            _celda('Ruta', 1, 4, padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4)),
          if (!lista) ...[
            _celda('Tipo', 5, 1, padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4)),
            _celda('Ext', 2, 1, padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4)),
            _celda('Tamaño', 3, 1, alignRight: true, padding: const EdgeInsets.only(left: 8, right: 16, top: 4, bottom: 4)),
            _celda('Modificado', 4, 2, padding: const EdgeInsets.only(left: 8, top: 4, bottom: 4)),
          ],
        ],
      ),
    );
  }

  Widget _celda(
    String etiqueta,
    int columna,
    int flex, {
    bool alignRight = false,
    EdgeInsetsGeometry padding = const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
  }) {
    final ordenada = estado.sortCol == columna;
    final flecha = ordenada ? (estado.ascending ? ' ▲' : ' ▼') : '';

    return Expanded(
      flex: flex,
      child: InkWell(
        onTap: () => notifier.setSort(columna),
        child: Padding(
          padding: padding,
          child: Text(
            '$etiqueta$flecha',
            textAlign: alignRight ? TextAlign.right : TextAlign.left,
            style: TextStyle(
              fontSize: 11,
              fontWeight: ordenada ? FontWeight.w700 : FontWeight.w600,
              color: ordenada ? AppColors.primary : AppColors.textSecondary,
            ),
          ),
        ),
      ),
    );
  }
}

/// Fila cuya página aún no ha llegado.
///
/// Ocupa exactamente lo mismo que una fila real para que el desplazamiento no
/// dé saltos mientras se carga.
class _FilaPendiente extends StatelessWidget {
  const _FilaPendiente({required this.index, required this.alto});

  final int index;
  final double alto;

  @override
  Widget build(BuildContext context) {
    return Container(
      height: alto,
      color: index.isEven ? AppColors.rowEven : AppColors.rowOdd,
      padding: const EdgeInsets.symmetric(horizontal: 16),
      alignment: Alignment.centerLeft,
      child: Container(
        height: 8,
        width: 180,
        decoration: BoxDecoration(
          color: AppColors.border,
          borderRadius: BorderRadius.circular(4),
        ),
      ),
    );
  }
}

/// Tarjeta de la vista de iconos: icono grande con el nombre debajo.
class _CasillaIcono extends StatelessWidget {
  const _CasillaIcono({
    required this.fila,
    required this.seleccionada,
    this.cortada = false,
    required this.onTapDown,
    required this.onContextMenu,
    required this.rutasArrastradas,
    this.onSoltarEnCarpeta,
  });

  final FileRow fila;
  final bool seleccionada;
  final bool cortada;
  final VoidCallback onTapDown;
  final void Function(Offset) onContextMenu;
  final Future<List<String>> Function() rutasArrastradas;
  final void Function(List<String> rutas, String destino)? onSoltarEnCarpeta;

  @override
  Widget build(BuildContext context) {
    final tarjeta = Container(
      decoration: BoxDecoration(
        color: seleccionada ? AppColors.selected : AppColors.rowOdd,
        borderRadius: BorderRadius.circular(8),
        border: Border.all(
          color: seleccionada ? AppColors.primary : AppColors.border,
          width: seleccionada ? 1.5 : 1,
        ),
      ),
      padding: const EdgeInsets.all(8),
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          Icon(
            fila.isDirectory ? Icons.folder_rounded : iconoExtension(fila.extension),
            size: 36,
            color: fila.isDirectory
                ? const Color(0xFFE5A93C)
                : AppColors.primary,
          ),
          const SizedBox(height: 6),
          Text(
            fila.name,
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
            textAlign: TextAlign.center,
            style: TextStyle(
              fontSize: 11,
              height: 1.2,
              fontWeight: seleccionada ? FontWeight.w600 : FontWeight.w400,
              color: fila.isDisconnected
                  ? AppColors.textDisabled
                  : AppColors.textPrimary,
            ),
          ),
        ],
      ),
    );

    final Widget widgetTarjeta = cortada
        ? Opacity(opacity: 0.5, child: tarjeta)
        : tarjeta;

    return RepaintBoundary(
      child: envolverConArrastre(
        fila: fila,
        rutasArrastradas: rutasArrastradas,
        onSoltarEnCarpeta: onSoltarEnCarpeta,
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTapDown: (_) => onTapDown(),
          onSecondaryTapDown: (d) => onContextMenu(d.globalPosition),
          child: widgetTarjeta,
        ),
      ),
    );
  }
}

/// Lugar reservado para una tarjeta cuya página aún no ha llegado.
class _CasillaPendiente extends StatelessWidget {
  const _CasillaPendiente();

  @override
  Widget build(BuildContext context) {
    return Container(
      decoration: BoxDecoration(
        color: AppColors.rowEven,
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: AppColors.border),
      ),
      child: const Icon(Icons.hourglass_empty_rounded, size: 22, color: AppColors.borderStrong),
    );
  }
}

class _Fila extends StatelessWidget {
  const _Fila({
    required this.fila,
    required this.index,
    required this.viewMode,
    required this.mostrarRuta,
    required this.seleccionada,
    this.cortada = false,
    required this.onTapDown,
    required this.onContextMenu,
    required this.rutasArrastradas,
    this.onSoltarEnCarpeta,
  });

  final FileRow fila;
  final int index;
  final ResultViewMode viewMode;
  final bool mostrarRuta;
  final bool seleccionada;
  final bool cortada;
  final VoidCallback onTapDown;
  final void Function(Offset) onContextMenu;
  final Future<List<String>> Function() rutasArrastradas;

  /// Cuando la fila es una carpeta: mueve `rutas` dentro de ella al soltarlas.
  final void Function(List<String> rutas, String destino)? onSoltarEnCarpeta;

  @override
  Widget build(BuildContext context) {
    final fondo = seleccionada
        ? AppColors.primary
        : (index.isEven ? AppColors.rowEven : AppColors.rowOdd);

    final colorTexto = seleccionada
        ? Colors.white
        : (fila.isDisconnected ? AppColors.textDisabled : AppColors.textPrimary);

    final colorSecundario = seleccionada
        ? Colors.white.withAlpha(200)
        : (fila.isDisconnected ? AppColors.textDisabled : AppColors.textSecondary);

    Widget filaWidget = Container(
      height: viewMode == ResultViewMode.compact ? 24.0 : 32.0,
      color: fondo,
      padding: EdgeInsets.symmetric(
        horizontal: viewMode == ResultViewMode.compact ? 8 : 16,
      ),
      child: Row(
        children: switch (viewMode) {
          ResultViewMode.details => _celdasDetalle(colorTexto, colorSecundario),
          ResultViewMode.list => _celdasLista(colorTexto, colorSecundario),
          ResultViewMode.compact => _celdasCompacta(colorTexto, colorSecundario),
          ResultViewMode.grid => _celdasCompacta(colorTexto, colorSecundario),
        },
      ),
    );

    if (cortada) {
      filaWidget = Opacity(opacity: 0.5, child: filaWidget);
    }

    return RepaintBoundary(
      child: envolverConArrastre(
        fila: fila,
        rutasArrastradas: rutasArrastradas,
        onSoltarEnCarpeta: onSoltarEnCarpeta,
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTapDown: (_) => onTapDown(),
          onSecondaryTapDown: (d) => onContextMenu(d.globalPosition),
          child: filaWidget,
        ),
      ),
    );
  }

  Widget _iconoFila() => Icon(
    fila.isDirectory ? Icons.folder_rounded : iconoExtension(fila.extension),
    size: 16,
    color: seleccionada
        ? Colors.white
        : (fila.isDirectory
            ? const Color(0xFFE5A93C)
            : AppColors.primary),
  );

  /// Vista de detalles: la tabla completa.
  List<Widget> _celdasDetalle(Color colorTexto, Color colorSecundario) {
    final flexNombre = mostrarRuta ? 5 : 8;
    return [
      Expanded(
        flex: flexNombre,
        child: Padding(
          padding: const EdgeInsets.only(right: 8),
          child: Row(
            children: [
              _iconoFila(),
              const SizedBox(width: 8),
              Expanded(
                child: Text(
                  fila.name,
                  overflow: TextOverflow.ellipsis,
                  style: _estiloNombre(colorTexto),
                ),
              ),
            ],
          ),
        ),
      ),
      if (mostrarRuta)
        _celda(fila.path, flex: 4, colorSecundario: colorSecundario, padding: const EdgeInsets.symmetric(horizontal: 8)),
      _celda(fila.tipoLabel, flex: 1, colorSecundario: colorSecundario, padding: const EdgeInsets.symmetric(horizontal: 8)),
      _celda(fila.extension, flex: 1, colorSecundario: colorSecundario, padding: const EdgeInsets.symmetric(horizontal: 8)),
      Expanded(
        flex: 1,
        child: Padding(
          padding: const EdgeInsets.only(left: 8, right: 16),
          child: Text(
            fila.formattedSize,
            textAlign: TextAlign.right,
            style: TextStyle(
              fontSize: 11,
              fontFamily: 'monospace',
              color: colorSecundario,
            ),
          ),
        ),
      ),
      Expanded(
        flex: 2,
        child: Padding(
          padding: const EdgeInsets.only(left: 8),
          child: Text(
            fila.formattedDate,
            style: TextStyle(fontSize: 11, color: colorSecundario),
          ),
        ),
      ),
    ];
  }

  /// Vista lista: icono, nombre y ruta (o solo nombre en modo carpeta).
  List<Widget> _celdasLista(Color colorTexto, Color colorSecundario) {
    final flexNombre = mostrarRuta ? 5 : 9;
    return [
      Expanded(
        flex: flexNombre,
        child: Padding(
          padding: const EdgeInsets.only(right: 8),
          child: Row(
            children: [
              _iconoFila(),
              const SizedBox(width: 8),
              Expanded(
                child: Text(
                  fila.name,
                  overflow: TextOverflow.ellipsis,
                  style: _estiloNombre(colorTexto),
                ),
              ),
            ],
          ),
        ),
      ),
      if (mostrarRuta)
        _celda(fila.path, flex: 4, colorSecundario: colorSecundario, padding: const EdgeInsets.symmetric(horizontal: 8)),
    ];
  }

  /// Vista compacta: icono y nombre, nada más.
  List<Widget> _celdasCompacta(Color colorTexto, Color colorSecundario) {
    return [
      _iconoFila(),
      const SizedBox(width: 6),
      Expanded(
        child: Text(
          fila.name,
          overflow: TextOverflow.ellipsis,
          style: TextStyle(
            fontSize: 11,
            fontWeight: seleccionada ? FontWeight.w600 : FontWeight.w400,
            color: colorTexto,
          ),
        ),
      ),
    ];
  }

  TextStyle _estiloNombre(Color colorTexto) => TextStyle(
    fontSize: 12,
    fontWeight: seleccionada ? FontWeight.w600 : FontWeight.w400,
    color: colorTexto,
  );

  Widget _celda(
    String texto, {
    required int flex,
    required Color colorSecundario,
    EdgeInsetsGeometry padding = const EdgeInsets.symmetric(horizontal: 8),
  }) =>
      Expanded(
        flex: flex,
        child: Padding(
          padding: padding,
          child: Text(
            texto,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(fontSize: 11, color: colorSecundario),
          ),
        ),
      );
}

/// Qué se ve cuando no hay filas.
///
/// Antes decía siempre «El motor de búsqueda está listo», incluso cuando el
/// motor no había podido abrir el índice. Ahora, si el problema es del motor, lo
/// dice con sus palabras.
class _EstadoVacio extends StatelessWidget {
  const _EstadoVacio({required this.estado});

  final SearchState estado;

  @override
  Widget build(BuildContext context) {
    final salud = estado.engine;
    if (estado.mode == ViewMode.browse) {
      return _mensaje(
        icono: Icons.folder_open_rounded,
        color: AppColors.primary.withAlpha(80),
        titulo: 'Esta carpeta está vacía',
        detalle: estado.browsePath.isEmpty ? 'No hay elementos para mostrar' : estado.browsePath,
      );
    }

    if (!salud.isOpen || salud.entryCount == 0) {
      return Center(
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 48),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              const SizedBox(
                width: 36,
                height: 36,
                child: CircularProgressIndicator(
                  strokeWidth: 3,
                  color: AppColors.primary,
                ),
              ),
              const SizedBox(height: 18),
              Text(
                estado.query.isEmpty
                    ? 'Indexando tu equipo por primera vez…'
                    : 'Indexando equipo… buscando «${estado.query}»',
                textAlign: TextAlign.center,
                style: const TextStyle(
                  fontSize: 16,
                  fontWeight: FontWeight.w600,
                  color: AppColors.textPrimary,
                ),
              ),
              const SizedBox(height: 8),
              Text(
                estado.query.isEmpty
                    ? 'BDJ Studio Search Pro está catalogando tus archivos en segundo plano para habilitar búsquedas instantáneas a 0 ms.\nEsto solo se realiza la primera vez y toma unos momentos.'
                    : 'El índice se está generando en segundo plano. En cuanto finalice, tus resultados aparecerán aquí automáticamente.',
                textAlign: TextAlign.center,
                style: const TextStyle(
                  fontSize: 12.5,
                  color: AppColors.textSecondary,
                  height: 1.45,
                ),
              ),
            ],
          ),
        ),
      );
    }

    if (estado.query.isEmpty) {
      return _mensaje(
        icono: Icons.search_rounded,
        color: AppColors.primary.withAlpha(80),
        titulo: 'Empieza a escribir para buscar',
        detalle: '${_conPuntos(salud.entryCount)} elementos indexados y listos.',
      );
    }

    return _mensaje(
      icono: Icons.search_off_rounded,
      color: AppColors.primary.withAlpha(80),
      titulo: 'No se encontraron resultados para «${estado.query}»',
      detalle: 'Prueba a cambiar los términos o la extensión (ej.: ext:wav)',
    );
  }

  Widget _mensaje({
    required IconData icono,
    required Color color,
    required String titulo,
    required String detalle,
  }) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 48),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(icono, size: 48, color: color),
            const SizedBox(height: 12),
            Text(
              titulo,
              textAlign: TextAlign.center,
              style: const TextStyle(
                fontSize: 15,
                fontWeight: FontWeight.w600,
                color: AppColors.textPrimary,
              ),
            ),
            const SizedBox(height: 6),
            Text(
              detalle,
              textAlign: TextAlign.center,
              style: const TextStyle(fontSize: 12, color: AppColors.textSecondary),
            ),
          ],
        ),
      ),
    );
  }

  static String _conPuntos(int n) => n.toString().replaceAllMapped(
    RegExp(r'(\d{1,3})(?=(\d{3})+(?!\d))'),
    (m) => '${m[1]}.',
  );
}

String _grupoDeFila(FileRow fila, GroupByMode mode) {
  if (mode == GroupByMode.type) {
    return fila.tipoLabel;
  } else if (mode == GroupByMode.date) {
    if (fila.mtime == 0) return 'Sin fecha';
    final dt = DateTime.fromMillisecondsSinceEpoch(fila.mtime * 1000);
    final now = DateTime.now();
    final diff = now.difference(dt);
    if (diff.inDays == 0) return 'Hoy';
    if (diff.inDays == 1) return 'Ayer';
    if (diff.inDays <= 7) return 'Esta semana';
    if (diff.inDays <= 30) return 'Este mes';
    if (diff.inDays <= 365) return 'Este año';
    return 'Hace más de un año';
  }
  return '';
}

class _CabeceraGrupo extends StatelessWidget {
  final String titulo;
  const _CabeceraGrupo({required this.titulo});

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
      margin: const EdgeInsets.only(top: 8, bottom: 4),
      decoration: const BoxDecoration(
        color: AppColors.surface,
        border: Border(
          top: BorderSide(color: AppColors.border),
          bottom: BorderSide(color: AppColors.border),
        ),
      ),
      child: Row(
        children: [
          const Icon(
            Icons.keyboard_arrow_down_rounded,
            size: 16,
            color: AppColors.textSecondary,
          ),
          const SizedBox(width: 6),
          Text(
            titulo.toUpperCase(),
            style: const TextStyle(
              fontSize: 11,
              fontWeight: FontWeight.bold,
              letterSpacing: 0.5,
              color: AppColors.textPrimary,
            ),
          ),
        ],
      ),
    );
  }
}
