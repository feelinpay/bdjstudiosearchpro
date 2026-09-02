import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:super_drag_and_drop/super_drag_and_drop.dart';

import '../../../core/theme/app_colors.dart';
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

  @override
  Widget build(BuildContext context) {
    final estado = ref.watch(searchProvider);
    final notifier = ref.read(searchProvider.notifier);
    final viewMode = estado.viewMode;
    final esGrid = viewMode == ResultViewMode.grid;

    final cuerpo = estado.rowCount == 0
        ? _EstadoVacio(estado: estado)
        : esGrid
            ? _grid(estado, notifier)
            : ListView.builder(
                controller: _scrollController,
                itemExtent: _altoFila(viewMode),
                // El total real, acotado a lo que el motor puede entregar
                // ordenado. La barra de desplazamiento deja de mentir.
                itemCount: estado.rowCount,
                itemBuilder: (context, index) {
                  final fila = notifier.rowAt(index);
                  if (fila == null) {
                    return _FilaPendiente(
                      index: index,
                      alto: _altoFila(viewMode),
                    );
                  }
                  return _Fila(
                    fila: fila,
                    index: index,
                    viewMode: viewMode,
                    seleccionada: estado.selection.contains(index),
                    onTap: () => _alPulsar(index),
                    onDoubleTap: () {
                      notifier.selectRow(index);
                      notifier.openSelected();
                    },
                    onContextMenu: (posicion) => _menuContextual(
                        context, notifier, ref.read(searchProvider), index, posicion),
                    rutasArrastradas: () => _rutasParaArrastrar(index),
                    onSoltarEnCarpeta: (rutas, destino) =>
                        _moverSoltados(rutas, destino),
                  );
                },
              );

    return Column(
      children: [
        if (!esGrid)
          _Cabecera(estado: estado, notifier: notifier, viewMode: viewMode),
        Expanded(
          // El fondo responde al clic derecho con su propio menú: como en el
          // Explorador, pulsar donde no hay un elemento crea carpetas o pega.
          // Las filas ceden la arena de gestos a su propio menú, así que este
          // menú solo aparece en los huecos.
          child: GestureDetector(
            behavior: HitTestBehavior.translucent,
            onSecondaryTapDown: (d) => _menuDeFondo(context, d.globalPosition),
            child: cuerpo,
          ),
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
  Widget _grid(SearchState estado, SearchNotifier notifier) {
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
          onTap: () => _alPulsar(index),
          onDoubleTap: () {
            notifier.selectRow(index);
            notifier.openSelected();
          },
          onContextMenu: (posicion) => _menuContextual(
              context, notifier, ref.read(searchProvider), index, posicion),
          rutasArrastradas: () => _rutasParaArrastrar(index),
          onSoltarEnCarpeta: (rutas, destino) => _moverSoltados(rutas, destino),
        );
      },
    );
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
    if (estado.selection.contains(index) && estado.selection.count > 1) {
      return notifier.selectedPaths(max: 5000);
    }
    final ruta = await notifier.pathOf(index);
    return ruta.isEmpty ? const [] : [ruta];
  }

  /// Mueve lo soltado dentro de una carpeta.
  ///
  /// Es el «arrastrar y soltar» interno: igual que en el explorador del
  /// sistema, soltar elementos sobre una carpeta los mueve a esa carpeta. La
  /// operación pasa por el motor (se puede deshacer y avisa al índice), así que
  /// la vista se refresca sola al terminar.
  Future<void> _moverSoltados(List<String> rutas, String destino) async {
    if (rutas.isEmpty || destino.isEmpty) return;
    // No hunde una carpeta dentro de sí misma.
    if (rutas.contains(destino)) return;
    final ops = ref.read(fileOpsProvider.notifier);
    await ops.moveTo(rutas, destino);
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

    final valor = await showMenu<String>(
      context: context,
      position: RelativeRect.fromLTRB(posicion.dx, posicion.dy, posicion.dx, posicion.dy),
      elevation: 4,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(4)),
      items: <PopupMenuEntry<String>>[
        _item('refresh', 'Actualizar'),
        _item('open', varios ? 'Abrir los seleccionados' : 'Abrir'),
        const PopupMenuDivider(),
        _item('cut', varios ? 'Cortar los seleccionados' : 'Cortar'),
        _item('copy', varios ? 'Copiar los seleccionados' : 'Copiar'),
        _item('duplicate', 'Duplicar'),
        if (!varios) _item('rename', 'Cambiar nombre'),
        const PopupMenuDivider(),
        _item('zip', 'Comprimir a archivo ZIP'),
        if (notifier.rowAt(index)?.fullPath.toLowerCase().endsWith('.zip') == true)
          _item('unzip', 'Descomprimir aquí'),
        const PopupMenuDivider(),
        _item('reveal', 'Mostrar la ubicación'),
        _item('copypath', varios ? 'Copiar las rutas' : 'Copiar la ruta'),
        _item('copyname', varios ? 'Copiar los nombres' : 'Copiar el nombre'),
        const PopupMenuDivider(),
        _item('selectall', 'Seleccionar todo'),
        _item('invert', 'Invertir la selección'),
        const PopupMenuDivider(),
        _item('trash', 'Enviar a la papelera'),
        _item('deleteforever', 'Eliminar permanentemente…'),
      ],
    );
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
        await ops.cutSelection();
      case 'copy':
        await ops.copySelection();
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
    final opsState = ref.read(fileOpsProvider);
    final destino = estado.browsePath;
    final enCarpeta = estado.mode == ViewMode.browse && destino.isNotEmpty;

    final valor = await showMenu<String>(
      context: context,
      position: RelativeRect.fromLTRB(posicion.dx, posicion.dy, posicion.dx, posicion.dy),
      elevation: 4,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(4)),
      items: <PopupMenuEntry<String>>[
        if (enCarpeta) ...[
          _item('newfolder', 'Nueva carpeta'),
          _item('newfile', 'Nuevo archivo'),
          _item('paste', 'Pegar', habilitado: opsState.clipboard.isNotEmpty),
          const PopupMenuDivider(),
        ],
        _item('selectall', 'Seleccionar todo'),
        _item('invert', 'Invertir la selección'),
      ],
    );
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
  final base = DragItemWidget(
    allowedOperations: () => [DropOperation.copy, DropOperation.move],
    dragItemProvider: (request) async {
      final rutas = await rutasArrastradas();
      if (rutas.isEmpty) return null;
      // Se entregan **referencias** al sistema: ni se copia, ni se mueve, ni
      // se crea ningún archivo temporal. El programa de destino recibe las
      // rutas reales y decide él qué hacer con ellas.
      final item = DragItem(localData: rutas.join('\n'));
      for (final r in rutas) {
        item.add(Formats.fileUri(Uri.file(r)));
      }
      return item;
    },
    child: DraggableWidget(child: child),
  );

  final soltar = onSoltarEnCarpeta;
  if (fila.isDirectory && soltar != null) {
    return DropRegion(
      formats: const [],
      onDropOver: (event) async {
        final local = event.session.items.isEmpty
            ? null
            : event.session.items.first.localData;
        if (local is String && local.trim().isNotEmpty) {
          return DropOperation.move;
        }
        return DropOperation.none;
      },
      onPerformDrop: (event) async {
        final local = event.session.items.isEmpty
            ? null
            : event.session.items.first.localData;
        if (local is String) {
          final rutas = local
              .split('\n')
              .map((r) => r.trim())
              .where((r) => r.isNotEmpty)
              .toList();
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
    return Container(
      height: 28,
      padding: const EdgeInsets.symmetric(horizontal: 16),
      decoration: const BoxDecoration(
        color: AppColors.surface,
        border: Border(bottom: BorderSide(color: AppColors.border)),
      ),
      child: Row(
        children: [
          _celda('Nombre', 0, lista ? 4 : 4),
          _celda('Ruta', 1, lista ? 3 : 3),
          if (!lista) ...[
            _celda('Tipo', 5, 1),
            _celda('Ext', 2, 1),
            _celda('Tamaño', 3, 1, alignRight: true),
            _celda('Modificado', 4, 2),
          ],
        ],
      ),
    );
  }

  Widget _celda(String etiqueta, int columna, int flex, {bool alignRight = false}) {
    final ordenada = estado.sortCol == columna;
    final flecha = ordenada ? (estado.ascending ? ' ▲' : ' ▼') : '';

    return Expanded(
      flex: flex,
      child: InkWell(
        onTap: () => notifier.setSort(columna),
        child: Padding(
          padding: const EdgeInsets.symmetric(vertical: 4),
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
    required this.onTap,
    required this.onDoubleTap,
    required this.onContextMenu,
    required this.rutasArrastradas,
    this.onSoltarEnCarpeta,
  });

  final FileRow fila;
  final bool seleccionada;
  final VoidCallback onTap;
  final VoidCallback onDoubleTap;
  final void Function(Offset) onContextMenu;
  final Future<List<String>> Function() rutasArrastradas;
  final void Function(List<String> rutas, String destino)? onSoltarEnCarpeta;

  @override
  Widget build(BuildContext context) {
    final contenedor = GestureDetector(
      onTap: onTap,
      onDoubleTap: onDoubleTap,
      onSecondaryTapDown: (d) => onContextMenu(d.globalPosition),
      child: Container(
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
      ),
    );

    return envolverConArrastre(
      child: contenedor,
      fila: fila,
      rutasArrastradas: rutasArrastradas,
      onSoltarEnCarpeta: onSoltarEnCarpeta,
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
    required this.seleccionada,
    required this.onTap,
    required this.onDoubleTap,
    required this.onContextMenu,
    required this.rutasArrastradas,
    this.onSoltarEnCarpeta,
  });

  final FileRow fila;
  final int index;
  final ResultViewMode viewMode;
  final bool seleccionada;
  final VoidCallback onTap;
  final VoidCallback onDoubleTap;
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

    return envolverConArrastre(
      fila: fila,
      rutasArrastradas: rutasArrastradas,
      onSoltarEnCarpeta: onSoltarEnCarpeta,
      child: GestureDetector(
        onTap: onTap,
        onDoubleTap: onDoubleTap,
        onSecondaryTapDown: (d) => onContextMenu(d.globalPosition),
        child: Container(
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
              // Las tarjetas usan `_CasillaIcono`; esta rama es solo para que
              // el `switch` siga siendo exhaustivo.
              ResultViewMode.grid => _celdasCompacta(colorTexto, colorSecundario),
            },
          ),
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
    return [
      Expanded(
        flex: 4,
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
      _celda(fila.path, flex: 3, colorSecundario: colorSecundario),
      _celda(fila.tipoLabel, flex: 1, colorSecundario: colorSecundario),
      _celda(fila.extension, flex: 1, colorSecundario: colorSecundario),
      Expanded(
        flex: 1,
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
      Expanded(
        flex: 2,
        child: Padding(
          padding: const EdgeInsets.only(left: 12),
          child: Text(
            fila.formattedDate,
            style: TextStyle(fontSize: 11, color: colorSecundario),
          ),
        ),
      ),
    ];
  }

  /// Vista lista: icono, nombre y ruta.
  List<Widget> _celdasLista(Color colorTexto, Color colorSecundario) {
    return [
      Expanded(
        flex: 4,
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
      _celda(fila.path, flex: 3, colorSecundario: colorSecundario),
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

  Widget _celda(String texto, {required int flex, required Color colorSecundario}) =>
      Expanded(
        flex: flex,
        child: Text(
          texto,
          overflow: TextOverflow.ellipsis,
          style: TextStyle(fontSize: 11, color: colorSecundario),
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

    if (!salud.isOpen) {
      final avisa = salud.problem.resolvesItself;
      return _mensaje(
        icono: avisa ? Icons.sync : Icons.error_outline,
        color: avisa ? Colors.orange : Colors.red,
        titulo: salud.shortLabel,
        detalle: salud.detail,
      );
    }

    if (estado.query.isEmpty && estado.mode == ViewMode.search) {
      return _mensaje(
        icono: Icons.search_rounded,
        color: AppColors.primary.withAlpha(80),
        titulo: 'Empieza a escribir para buscar',
        detalle:
            '${_conPuntos(salud.entryCount)} elementos indexados y listos.',
      );
    }

    if (estado.mode == ViewMode.browse) {
      return _mensaje(
        icono: Icons.folder_open_rounded,
        color: AppColors.primary.withAlpha(80),
        titulo: 'Esta carpeta está vacía',
        detalle: estado.browsePath,
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
