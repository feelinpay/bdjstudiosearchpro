import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import '../../../core/theme/app_colors.dart';
import '../models/engine_health.dart';
import '../providers/search_provider.dart';

/// Barra inferior: cuántos resultados, qué hay seleccionado y cómo está el motor.
///
/// La versión anterior solo sabía pintar «Índice listo» o «Cargando motor…», así
/// que **cualquier** fallo —falta el servicio, el índice es de otra versión, no
/// hay permisos— acababa mostrándose como si estuviera cargando algo. No estaba
/// cargando nada: estaba fallando en bucle, y el usuario no tenía forma de
/// saberlo.
class SearchStatusBar extends ConsumerWidget {
  const SearchStatusBar({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final estado = ref.watch(searchProvider);
    final notifier = ref.read(searchProvider.notifier);
    final salud = estado.engine;

    final seleccion = estado.selection.count;
    final filaActual = notifier.rowAt(estado.selection.cursor);

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
          Text(
            _resumen(estado.totalCount, seleccion, estado.elapsedMs,
                truncado: estado.isTruncated),
            style: const TextStyle(fontSize: 11, color: AppColors.textSecondary),
          ),

          if (filaActual != null)
            Expanded(
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 16),
                child: Text(
                  filaActual.fullPath,
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

          _EstadoDelMotor(salud: salud),
        ],
      ),
    );
  }

  String _resumen(int total, int seleccion, int ms, {required bool truncado}) {
    final aviso = truncado
        ? ' · se muestran las primeras ${_formatNumber(SearchState.maxAddressableRows)}'
        : '';
    final objetos = '${_formatNumber(total)} objetos ($ms ms)$aviso';
    if (seleccion <= 0) return objetos;
    if (seleccion == 1) return '$objetos · 1 seleccionado';
    return '$objetos · ${_formatNumber(seleccion)} seleccionados';
  }

  static String _formatNumber(int number) {
    return number.toString().replaceAllMapped(
      RegExp(r'(\d{1,3})(?=(\d{3})+(?!\d))'),
      (m) => '${m[1]}.',
    );
  }
}

/// El indicador de la derecha, con el motivo real detrás.
class _EstadoDelMotor extends StatelessWidget {
  const _EstadoDelMotor({required this.salud});

  final EngineHealth salud;

  @override
  Widget build(BuildContext context) {
    late final Color color;
    late final IconData icono;
    if (salud.isOpen) {
      color = AppColors.textSecondary;
      icono = Icons.check_circle_outline;
    } else if (salud.isChecking || !salud.isStable) {
      // Cualquier estado transitorio (motor todavía sin responder, o acaba de
      // llegar un veredicto y aún no pasó el periodo de gracia): un aviso, no
      // un error. Sin esto, el primer arranque destellaba rojo durante varios
      // fotogramas antes de que el servicio alcanzara a publicar el índice.
      color = AppColors.textSecondary;
      icono = Icons.sync;
    } else if (salud.problem == IndexProblem.denied) {
      color = Colors.red;
      icono = Icons.lock_outline;
    } else if (salud.problem.resolvesItself) {
      color = AppColors.textSecondary;
      icono = Icons.sync;
    } else {
      color = Colors.red;
      icono = Icons.error_outline;
    }

    return Tooltip(
      message: salud.detail.isEmpty ? salud.shortLabel : salud.detail,
      waitDuration: const Duration(milliseconds: 300),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(icono, size: 12, color: color),
          const SizedBox(width: 4),
          Text(
            salud.shortLabel,
            style: TextStyle(
              fontSize: 11,
              fontWeight: FontWeight.w500,
              color: color,
            ),
          ),
        ],
      ),
    );
  }
}
