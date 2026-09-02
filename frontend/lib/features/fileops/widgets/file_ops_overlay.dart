import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../core/ffi/api.dart' as ffi;
import '../../../core/theme/app_colors.dart';
import '../providers/file_ops_provider.dart';

/// Panel de operaciones en curso y diálogo de conflictos.
///
/// Va superpuesto a la tabla, abajo a la derecha, y solo aparece cuando hay
/// trabajo. La interfaz nunca se queda esperando a que termine una copia: lo que
/// se ve aquí es el avance de unos hilos que trabajan por su cuenta.
class FileOpsOverlay extends ConsumerWidget {
  const FileOpsOverlay({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final estado = ref.watch(fileOpsProvider);
    final notifier = ref.read(fileOpsProvider.notifier);

    final conflicto = estado.pendingConflict;
    if (conflicto != null) {
      // El diálogo se pide en el siguiente fotograma: mostrar una ruta modal
      // durante el `build` es un error en Flutter.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        _mostrarConflicto(context, notifier, conflicto);
      });
    }

    final activas = estado.operations
        .where((op) => op.state != 4 && op.state != 5)
        .toList();
    if (activas.isEmpty) return const SizedBox.shrink();

    return Positioned(
      right: 16,
      bottom: 16,
      width: 360,
      child: Material(
        elevation: 8,
        borderRadius: BorderRadius.circular(8),
        color: AppColors.surface,
        child: Container(
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(8),
            border: Border.all(color: AppColors.border),
          ),
          padding: const EdgeInsets.all(12),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              for (final op in activas.take(4)) _Operacion(op: op, notifier: notifier),
              if (activas.length > 4)
                Padding(
                  padding: const EdgeInsets.only(top: 6),
                  child: Text(
                    'y ${activas.length - 4} más…',
                    style: const TextStyle(
                        fontSize: 11, color: AppColors.textSecondary),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }

  static bool _dialogoAbierto = false;

  void _mostrarConflicto(
    BuildContext context,
    FileOpsNotifier notifier,
    ffi.FileOpFfi op,
  ) {
    if (_dialogoAbierto || !context.mounted) return;
    _dialogoAbierto = true;

    var aplicarATodos = false;

    showDialog<void>(
      context: context,
      barrierDismissible: false,
      builder: (ctx) => StatefulBuilder(
        builder: (ctx, setLocalState) => AlertDialog(
          title: const Text('El archivo ya existe'),
          content: SizedBox(
            width: 460,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                const Text(
                  'Ya hay un archivo con ese nombre en el destino.',
                  style: TextStyle(fontSize: 13),
                ),
                const SizedBox(height: 16),
                _Comparacion(
                  titulo: 'El que se copia',
                  ruta: op.conflictSource,
                  bytes: op.conflictSourceSize.toInt(),
                  mtime: op.conflictSourceMtime,
                  destacado: true,
                ),
                const SizedBox(height: 10),
                _Comparacion(
                  titulo: 'El que ya está ahí',
                  ruta: op.conflictDestination,
                  bytes: op.conflictDestinationSize.toInt(),
                  mtime: op.conflictDestinationMtime,
                  destacado: false,
                ),
                const SizedBox(height: 12),
                CheckboxListTile(
                  value: aplicarATodos,
                  onChanged: (v) => setLocalState(() => aplicarATodos = v ?? false),
                  dense: true,
                  contentPadding: EdgeInsets.zero,
                  controlAffinity: ListTileControlAffinity.leading,
                  title: const Text(
                    'Hacer lo mismo con el resto',
                    style: TextStyle(fontSize: 12),
                  ),
                ),
              ],
            ),
          ),
          actions: [
            TextButton(
              onPressed: () => _responder(
                  ctx, notifier, op.id, ConflictDecision.cancel, aplicarATodos),
              child: const Text('Cancelar todo'),
            ),
            TextButton(
              onPressed: () => _responder(
                  ctx, notifier, op.id, ConflictDecision.skip, aplicarATodos),
              child: const Text('Omitir'),
            ),
            TextButton(
              onPressed: () => _responder(
                  ctx, notifier, op.id, ConflictDecision.keepBoth, aplicarATodos),
              child: const Text('Mantener ambos'),
            ),
            // Sobrescribir va en último lugar y sin destacar: es la única opción
            // que no se puede deshacer.
            TextButton(
              style: TextButton.styleFrom(foregroundColor: Colors.red),
              onPressed: () => _responder(
                  ctx, notifier, op.id, ConflictDecision.overwrite, aplicarATodos),
              child: const Text('Reemplazar'),
            ),
          ],
        ),
      ),
    ).whenComplete(() => _dialogoAbierto = false);
  }

  void _responder(
    BuildContext ctx,
    FileOpsNotifier notifier,
    BigInt id,
    int decision,
    bool aplicarATodos,
  ) {
    notifier.resolveConflict(id, decision, aplicarATodos);
    Navigator.of(ctx).pop();
  }
}

class _Comparacion extends StatelessWidget {
  const _Comparacion({
    required this.titulo,
    required this.ruta,
    required this.bytes,
    required this.mtime,
    required this.destacado,
  });

  final String titulo;
  final String ruta;
  final int bytes;
  final int mtime;
  final bool destacado;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.all(10),
      decoration: BoxDecoration(
        color: destacado ? AppColors.primary.withAlpha(18) : AppColors.rowEven,
        borderRadius: BorderRadius.circular(6),
        border: Border.all(color: AppColors.border),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            titulo,
            style: const TextStyle(
              fontSize: 11,
              fontWeight: FontWeight.w700,
              color: AppColors.textSecondary,
            ),
          ),
          const SizedBox(height: 3),
          Text(
            ruta,
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
            style: const TextStyle(fontSize: 12, color: AppColors.textPrimary),
          ),
          const SizedBox(height: 3),
          Text(
            '${_tam(bytes)} · ${_fecha(mtime)}',
            style: const TextStyle(fontSize: 11, color: AppColors.textSecondary),
          ),
        ],
      ),
    );
  }

  static String _tam(int bytes) {
    if (bytes < 1024) return '$bytes B';
    const unidades = ['KB', 'MB', 'GB', 'TB'];
    var valor = bytes / 1024;
    var i = 0;
    while (valor >= 1024 && i < unidades.length - 1) {
      valor /= 1024;
      i++;
    }
    return '${valor.toStringAsFixed(valor >= 100 ? 0 : 1)} ${unidades[i]}';
  }

  static String _fecha(int epochSecs) {
    if (epochSecs == 0) return 'sin fecha';
    final d = DateTime.fromMillisecondsSinceEpoch(epochSecs * 1000);
    String dos(int n) => n.toString().padLeft(2, '0');
    return '${dos(d.day)}/${dos(d.month)}/${d.year} ${dos(d.hour)}:${dos(d.minute)}';
  }
}

class _Operacion extends StatelessWidget {
  const _Operacion({required this.op, required this.notifier});

  final ffi.FileOpFfi op;
  final FileOpsNotifier notifier;

  @override
  Widget build(BuildContext context) {
    final total = op.totalBytes.toInt();
    final hecho = op.doneBytes.toInt();
    final porItems = op.totalItems.toInt() > 0
        ? op.doneItems.toInt() / op.totalItems.toInt()
        : 0.0;
    final progreso = total > 0 ? (hecho / total).clamp(0.0, 1.0) : porItems.clamp(0.0, 1.0);

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 6),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Expanded(
                child: Text(
                  _titulo(op),
                  style: const TextStyle(
                    fontSize: 12,
                    fontWeight: FontWeight.w600,
                    color: AppColors.textPrimary,
                  ),
                ),
              ),
              if (op.state == OpState.running || op.state == OpState.planning)
                IconButton(
                  icon: const Icon(Icons.close_rounded, size: 15),
                  tooltip: 'Cancelar',
                  splashRadius: 14,
                  padding: EdgeInsets.zero,
                  constraints: const BoxConstraints(minWidth: 24, minHeight: 24),
                  color: AppColors.textSecondary,
                  onPressed: () => notifier.cancel(op.id),
                ),
            ],
          ),
          const SizedBox(height: 4),
          ClipRRect(
            borderRadius: BorderRadius.circular(3),
            child: LinearProgressIndicator(
              value: op.state == OpState.planning ? null : progreso,
              minHeight: 4,
              backgroundColor: AppColors.border,
              color: op.state == OpState.failed ? Colors.red : AppColors.primary,
            ),
          ),
          const SizedBox(height: 4),
          Text(
            _detalle(op),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: const TextStyle(fontSize: 10, color: AppColors.textSecondary),
          ),
          if (op.errors.isNotEmpty)
            Padding(
              padding: const EdgeInsets.only(top: 3),
              child: Text(
                op.errors.first,
                maxLines: 2,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(fontSize: 10, color: Colors.red),
              ),
            ),
        ],
      ),
    );
  }

  String _titulo(ffi.FileOpFfi op) {
    final verbo = switch (op.kind) {
      0 => 'Creando carpeta',
      1 => 'Renombrando',
      2 => 'Copiando',
      3 => 'Moviendo',
      4 => 'Duplicando',
      5 => 'Enviando a la papelera',
      7 => 'Eliminando permanentemente',
      8 => 'Creando archivo',
      _ => 'Restaurando desde la papelera',
    };
    final n = op.totalItems.toInt();
    return n > 1 ? '$verbo $n elementos' : verbo;
  }

  String _detalle(ffi.FileOpFfi op) {
    if (op.state == OpState.planning) return 'Calculando…';
    if (op.state == OpState.waitingConflict) return 'Esperando tu respuesta…';
    if (op.state == OpState.cancelling) return 'Cancelando…';
    if (op.state == OpState.failed) return 'Terminó con errores';
    final nombre = op.current.isEmpty
        ? ''
        : op.current.split(RegExp(r'[\\/]')).last;
    return '${op.doneItems} de ${op.totalItems} · $nombre';
  }
}
