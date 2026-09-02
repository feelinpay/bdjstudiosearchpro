import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:file_selector/file_selector.dart';
import '../../../core/indexador.dart';
import '../../fileops/file_ops_dialogs.dart';
import '../../fileops/providers/file_ops_provider.dart';
import '../../../core/theme/app_colors.dart';
import '../providers/favorites_provider.dart';
import '../providers/search_provider.dart';
import '../models/view_mode.dart';

class TopMenuBar extends ConsumerWidget {
  const TopMenuBar({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final searchState = ref.watch(searchProvider);
    final searchNotifier = ref.read(searchProvider.notifier);
    final opsState = ref.watch(fileOpsProvider);
    final ops = ref.read(fileOpsProvider.notifier);
    final haySeleccion = searchState.selection.isNotEmpty;

    return MenuBar(
      children: [
SubmenuButton(
              menuChildren: [
                MenuItemButton(
                  shortcut: const SingleActivator(LogicalKeyboardKey.f5),
                  onPressed: () => searchNotifier.refreshNow(),
                  child: const Text('Actualizar'),
                ),
                const Divider(),
                MenuItemButton(
                  shortcut: const SingleActivator(LogicalKeyboardKey.keyS, control: true),
                  onPressed: () => _exportCsv(context, searchNotifier),
                  child: const Text('Exportar...'),
                ),
                const Divider(),
                const MenuItemButton(
                  shortcut: SingleActivator(LogicalKeyboardKey.keyQ, control: true),
                  onPressed: terminarApp,
                  child: Text('Salir'),
                ),
              ],
              child: const Text('Archivo'),
            ),
        SubmenuButton(
          menuChildren: [
            MenuItemButton(
              shortcut: const SingleActivator(LogicalKeyboardKey.keyA, control: true),
              onPressed: searchNotifier.selectAll,
              child: const Text('Seleccionar todo'),
            ),
            MenuItemButton(
              onPressed: searchNotifier.invertSelection,
              child: const Text('Invertir la selección'),
            ),
            MenuItemButton(
              onPressed: searchNotifier.clearSelection,
              child: const Text('Deseleccionar'),
            ),
            const Divider(),
            MenuItemButton(
              shortcut: const SingleActivator(LogicalKeyboardKey.keyC, control: true),
              onPressed: haySeleccion ? ops.copySelection : null,
              child: const Text('Copiar'),
            ),
            MenuItemButton(
              shortcut: const SingleActivator(LogicalKeyboardKey.keyX, control: true),
              onPressed: haySeleccion ? ops.cutSelection : null,
              child: const Text('Cortar'),
            ),
            MenuItemButton(
              shortcut: const SingleActivator(LogicalKeyboardKey.keyV, control: true),
              onPressed: opsState.clipboard.isNotEmpty &&
                      searchState.mode == ViewMode.browse
                  ? () => ops.pasteInto(searchState.browsePath)
                  : null,
              child: const Text('Pegar'),
            ),
            const Divider(),
            MenuItemButton(
              onPressed: searchState.mode == ViewMode.browse
                  ? () => nuevaCarpetaDialog(context, ops, searchState.browsePath)
                  : null,
              child: const Text('Nueva carpeta'),
            ),
            MenuItemButton(
              onPressed: searchState.mode == ViewMode.browse
                  ? () => nuevoArchivoDialog(context, ops, searchState.browsePath)
                  : null,
              child: const Text('Nuevo archivo'),
            ),
            MenuItemButton(
              shortcut: const SingleActivator(LogicalKeyboardKey.f2),
              onPressed: haySeleccion
                  ? () => renombrarDialog(context, ref, ops)
                  : null,
              child: const Text('Cambiar nombre'),
            ),
            MenuItemButton(
              onPressed: haySeleccion ? ops.duplicateSelection : null,
              child: const Text('Duplicar'),
            ),
            MenuItemButton(
              shortcut: const SingleActivator(LogicalKeyboardKey.delete),
              onPressed: haySeleccion
                  ? () => confirmarEnviarALaPapelera(
                      context, ops, searchState.selection.count)
                  : null,
              child: const Text('Enviar a la papelera'),
            ),
            MenuItemButton(
              onPressed: haySeleccion
                  ? () => eliminarPermanentemente(
                      context, ops, searchState.selection.count)
                  : null,
              child: const Text('Eliminar permanentemente…'),
            ),
            const Divider(),
            MenuItemButton(
              shortcut: const SingleActivator(LogicalKeyboardKey.keyZ, control: true),
              onPressed: opsState.canUndo ? ops.undoLast : null,
              child: const Text('Deshacer'),
            ),
            MenuItemButton(
              shortcut:
                  const SingleActivator(LogicalKeyboardKey.keyZ, control: true, shift: true),
              onPressed: opsState.canRedo ? ops.redoLast : null,
              child: const Text('Rehacer'),
            ),
          ],
          child: const Text('Edición'),
        ),
        SubmenuButton(
          menuChildren: [
            for (final modo in const [
              ResultViewMode.details,
              ResultViewMode.list,
              ResultViewMode.compact,
              ResultViewMode.grid,
            ])
              CheckboxMenuButton(
                value: searchState.viewMode == modo,
                onChanged: (_) => searchNotifier.setResultView(modo),
                child: Text(
                  modo.label,
                  style: const TextStyle(fontSize: 13),
                ),
              ),
          ],
          child: const Text('Ver'),
        ),
        SubmenuButton(
          menuChildren: [
            CheckboxMenuButton(
              value: searchState.isCaseSensitive,
              onChanged: (_) => searchNotifier.toggleCaseSensitive(),
              shortcut: const SingleActivator(LogicalKeyboardKey.keyI, control: true),
              child: const Text('Coincidir Letras'),
            ),
            CheckboxMenuButton(
              value: searchState.isPathMatch,
              onChanged: (_) => searchNotifier.togglePathMatch(),
              shortcut: const SingleActivator(LogicalKeyboardKey.keyU, control: true),
              child: const Text('Coincidir Ubicación'),
            ),
            CheckboxMenuButton(
              value: searchState.isRegex,
              onChanged: (_) => searchNotifier.toggleRegex(),
              shortcut: const SingleActivator(LogicalKeyboardKey.keyR, control: true),
              child: const Text('Habilitar Regex'),
            ),
            const Divider(),
            SubmenuButton(
              menuChildren: [
                _buildFilterItem(searchNotifier, searchState.activeFilter, 'Todos'),
                _buildFilterItem(searchNotifier, searchState.activeFilter, 'Audio'),
                _buildFilterItem(searchNotifier, searchState.activeFilter, 'Comprimidos'),
                _buildFilterItem(searchNotifier, searchState.activeFilter, 'Documentos'),
                _buildFilterItem(searchNotifier, searchState.activeFilter, 'Ejecutables'),
                _buildFilterItem(searchNotifier, searchState.activeFilter, 'Carpetas'),
                _buildFilterItem(searchNotifier, searchState.activeFilter, 'Imagen'),
                _buildFilterItem(searchNotifier, searchState.activeFilter, 'Vídeo'),
              ],
              child: const Text('Filtros Rápidos'),
            ),
      ],
      child: const Text('Búsqueda'),
    ),
    SubmenuButton(
      menuChildren: [
        MenuItemButton(
          onPressed: () => _anadirMarcador(context, ref, searchState),
          child: const Text('Añadir a Marcadores'),
        ),
        MenuItemButton(
          onPressed: () => _organizarMarcadores(context, ref),
          child: const Text('Organizar Marcadores...'),
        ),
      ],
      child: const Text('Marcadores'),
    ),
  ],
);
  }

  /// Añade la carpeta actual a los marcadores.
  void _anadirMarcador(
    BuildContext context,
    WidgetRef ref,
    SearchState searchState,
  ) {
    final path = searchState.mode == ViewMode.browse
        ? searchState.browsePath
        : '';
    if (path.isEmpty) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(
            content: Text('Abre una carpeta para añadirla a Marcadores')),
      );
      return;
    }
    ref.read(favoritesProvider.notifier).add(path);
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text('Marcador añadido: $path')),
    );
  }

  /// Diálogo para ver, abrir, reordenar y eliminar los marcadores.
  Future<void> _organizarMarcadores(BuildContext context, WidgetRef ref) async {
    await showDialog<void>(
      context: context,
      builder: (ctx) => Consumer(
        builder: (ctx, r, _) {
          final lista = r.watch(favoritesProvider);
          final notifier = r.read(favoritesProvider.notifier);
          return AlertDialog(
            title: const Text('Organizar Marcadores'),
            content: SizedBox(
              width: 420,
              height: 320,
              child: Column(
                children: [
                  const Text(
                    'Pulsa en un marcador para abrirlo. Usa los botones para '
                    'reordenar o eliminar.',
                    style: TextStyle(fontSize: 12, color: AppColors.textSecondary),
                  ),
                  const SizedBox(height: 8),
                  Expanded(
                    child: lista.isEmpty
                        ? const Center(
                            child: Text('Sin marcadores todavía',
                                style: TextStyle(color: AppColors.textSecondary)),
                          )
                        : ListView.separated(
                            itemCount: lista.length,
                            separatorBuilder: (_, _) =>
                                const Divider(height: 1, color: AppColors.border),
                            itemBuilder: (context, index) {
                              final path = lista[index];
                              final name = path
                                  .split(RegExp(r'[\\/]'))
                                  .lastWhere((s) => s.isNotEmpty,
                                      orElse: () => path);
                              return ListTile(
                                dense: true,
                                leading: const Icon(Icons.star,
                                    size: 16, color: Colors.amber),
                                title: Text(name,
                                    overflow: TextOverflow.ellipsis,
                                    style: const TextStyle(fontSize: 13)),
                                subtitle: Text(path,
                                    overflow: TextOverflow.ellipsis,
                                    style: const TextStyle(
                                        fontSize: 11,
                                        color: AppColors.textSecondary)),
                                trailing: Row(
                                  mainAxisSize: MainAxisSize.min,
                                  children: [
                                    IconButton(
                                      icon: const Icon(Icons.arrow_upward,
                                          size: 15),
                                      tooltip: 'Subir',
                                      onPressed: index > 0
                                          ? () => notifier.moveUp(path)
                                          : null,
                                    ),
                                    IconButton(
                                      icon: const Icon(Icons.arrow_downward,
                                          size: 15),
                                      tooltip: 'Bajar',
                                      onPressed:
                                          index < lista.length - 1
                                              ? () => notifier.moveDown(path)
                                              : null,
                                    ),
                                    IconButton(
                                      icon: const Icon(Icons.delete_outline,
                                          size: 15,
                                          color: AppColors.textSecondary),
                                      tooltip: 'Eliminar',
                                      onPressed: () => notifier.remove(path),
                                    ),
                                  ],
                                ),
                                onTap: () {
                                  ref.read(searchProvider.notifier)
                                      .openFolder(path);
                                  Navigator.of(ctx).pop();
                                },
                              );
                            },
                          ),
                  ),
                ],
              ),
            ),
            actions: [
              TextButton(
                onPressed: () => Navigator.of(ctx).pop(),
                child: const Text('Cerrar'),
              ),
            ],
          );
        },
      ),
    );
  }

  Widget _buildFilterItem(SearchNotifier notifier, String active, String filterName) {
    return RadioMenuButton<String>(
      value: filterName,
      groupValue: active,
      onChanged: (val) {
        if (val != null) notifier.setFilter(val);
      },
      child: Text(filterName == 'Todos' ? 'Buscar Todos los archivos y carpetas' : 'Buscar solo $filterName'),
    );
  }

  Future<void> _exportCsv(BuildContext context, SearchNotifier notifier) async {
    const String fileName = 'resultados_busqueda.csv';
    final FileSaveLocation? result = await getSaveLocation(
      suggestedName: fileName,
      acceptedTypeGroups: [
        const XTypeGroup(label: 'CSV', extensions: ['csv']),
      ],
    );
    if (result == null) return;

    final csv = await notifier.buildCsv();
    await File(result.path).writeAsString(csv);

    if (context.mounted) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('Exportado a ${result.path}')),
      );
    }
  }
}
