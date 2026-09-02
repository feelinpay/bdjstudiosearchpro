import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:super_drag_and_drop/super_drag_and_drop.dart';
import '../../../core/theme/app_colors.dart';
import '../../../core/ffi/api.dart' as ffi;
import '../../fileops/providers/file_ops_provider.dart';
import '../providers/favorites_provider.dart';
import '../providers/recent_folders_provider.dart';
import '../providers/search_provider.dart';

/// Panel lateral de navegación con Marcadores (favoritos) y equipo/volúmenes.
class SidebarTree extends ConsumerStatefulWidget {
  const SidebarTree({super.key});

  @override
  ConsumerState<SidebarTree> createState() => _SidebarTreeState();
}

class _SidebarTreeState extends ConsumerState<SidebarTree> {
  ffi.ServiceStatusFfi? _serviceStatus;

  /// Subdirectorios conocidos de cada carpeta rama del árbol (carga perezosa).
  final Map<String, List<String>> _subdirs = {};

  /// Carpetas cuya rama está desplegada en el árbol.
  final Set<String> _expandidas = {};

  @override
  void initState() {
    super.initState();
    _loadServiceStatus();
  }

  Future<void> _loadServiceStatus() async {
    try {
      final status = await ffi.serviceStatus();
      if (mounted) {
        setState(() {
          _serviceStatus = status;
        });
      }
    } catch (e) {
      // Los volúmenes siguen operativos sin el estado del servicio; basta con
      // que el error no se trague en silencio.
      debugPrint('No se pudo consultar el estado del servicio: $e');
    }
  }

  /// Mueve lo soltado dentro de la carpeta `destino`.
  Future<void> _moverSoltados(List<String> rutas, String destino) async {
    if (rutas.isEmpty || destino.isEmpty) return;
    if (rutas.contains(destino)) return;
    final ops = ref.read(fileOpsProvider.notifier);
    await ops.moveTo(rutas, destino);
  }

  /// Envuelve una carpeta del árbol lateral para aceptar sueltas (mover).
  Widget _aceptaSuelta(Widget child, String destino) {
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
          await _moverSoltados(rutas, destino);
        }
      },
      child: child,
    );
  }

  /// Expande o contrae la rama `path`, cargando sus hijos la primera vez.
  Future<void> _alternarRama(String path) async {
    if (_expandidas.contains(path)) {
      setState(() => _expandidas.remove(path));
      return;
    }
    if (!_subdirs.containsKey(path)) {
      List<String> hijos;
      try {
        hijos = await ffi.browseSubdirs(path: path);
      } catch (_) {
        hijos = const [];
      }
      if (!mounted) return;
      setState(() {
        _subdirs[path] = hijos;
        if (hijos.isNotEmpty) _expandidas.add(path);
      });
      return;
    }
    setState(() => _expandidas.add(path));
  }

  /// Las filas de una rama del árbol: la carpeta y, si está desplegada, la de
  /// cada uno de sus subdirectorios (recursivo).
  List<Widget> _ramas(String path, String nombre, int profundidad) {
    final expandida = _expandidas.contains(path);
    final widget = _filaArbol(path, nombre, profundidad, expandida);
    if (!expandida) return [widget];
    final hijos = _subdirs[path] ?? const <String>[];
    return [
      widget,
      for (final hijo in hijos)
        ..._ramas(
          hijo,
          hijo
              .split(RegExp(r'[\\/]'))
              .lastWhere((s) => s.isNotEmpty, orElse: () => hijo),
          profundidad + 1,
        ),
    ];
  }

  /// Una fila del árbol jerárquico: icono de carpeta, flecha de expandir y
  /// múltiples soltar-para-mover.
  Widget _filaArbol(
      String path, String nombre, int profundidad, bool expandida) {
    final searchState = ref.read(searchProvider);
    final notifier = ref.read(searchProvider.notifier);
    final isSelected = searchState.browsePath == path;

    final flecha = InkWell(
      onTap: () => _alternarRama(path),
      child: Padding(
        padding: const EdgeInsets.all(2),
        child: Icon(
          expandida ? Icons.arrow_drop_down : Icons.arrow_right,
          size: 18,
          color: AppColors.textSecondary,
        ),
      ),
    );

    final tile = InkWell(
      onTap: () => notifier.openFolder(path),
      child: Padding(
        padding: EdgeInsets.only(
          left: 6.0 + profundidad * 12.0,
          right: 4,
          top: 6,
          bottom: 6,
        ),
        child: Row(
          children: [
            Icon(
              Icons.folder_rounded,
              size: 16,
              color: isSelected ? AppColors.primary : const Color(0xFFE5A93C),
            ),
            const SizedBox(width: 6),
            Expanded(
              child: Text(
                nombre,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontSize: 12,
                  fontWeight: isSelected ? FontWeight.bold : FontWeight.normal,
                  color: isSelected ? AppColors.primary : AppColors.textPrimary,
                ),
              ),
            ),
          ],
        ),
      ),
    );

    return _aceptaSuelta(
      Row(
        children: [
          flecha,
          Expanded(child: tile),
        ],
      ),
      path,
    );
  }

  @override
  Widget build(BuildContext context) {
    final searchState = ref.watch(searchProvider);
    final notifier = ref.read(searchProvider.notifier);
    final favorites = ref.watch(favoritesProvider);
    final favoritesNotifier = ref.read(favoritesProvider.notifier);
    final recentFolders = ref.watch(recentFoldersProvider);
    final serviceStatus = _serviceStatus;

    return Container(
      width: 220,
      decoration: const BoxDecoration(
        color: AppColors.surface,
        border: Border(right: BorderSide(color: AppColors.border)),
      ),
      child: Material(
        color: Colors.transparent,
        child: ListView(
          padding: const EdgeInsets.symmetric(vertical: 4),
          children: [
          // Encabezado de Acceso Rápido
          const Padding(
            padding: EdgeInsets.symmetric(horizontal: 12, vertical: 8),
            child: Text(
              'ACCESO RÁPIDO',
              style: TextStyle(
                fontSize: 11,
                fontWeight: FontWeight.bold,
                color: AppColors.textSecondary,
              ),
            ),
          ),
          ..._obtenerAccesosRapidos().map((item) {
            final isSelected = searchState.browsePath == item.ruta ||
                (searchState.browsePath.startsWith(item.ruta) &&
                    item.ruta.endsWith(Platform.pathSeparator));
            return _aceptaSuelta(
              ListTile(
                dense: true,
                visualDensity: VisualDensity.compact,
                leading: Icon(item.icono, size: 16, color: item.color),
                title: Text(
                  item.nombre,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontSize: 12,
                    fontWeight: isSelected ? FontWeight.bold : FontWeight.normal,
                    color: isSelected ? AppColors.primary : AppColors.textPrimary,
                  ),
                ),
                onTap: () => notifier.openFolder(item.ruta),
              ),
              item.ruta,
            );
          }),
          const Divider(height: 16, color: AppColors.border),

          // Encabezado de Marcadores
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
            child: Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              children: [
                const Text(
                  'MIS MARCADORES',
                  style: TextStyle(
                    fontSize: 11,
                    fontWeight: FontWeight.bold,
                    color: AppColors.textSecondary,
                  ),
                ),
                IconButton(
                  icon: const Icon(Icons.add,
                      size: 16, color: AppColors.textSecondary),
                  onPressed: () {
                    if (searchState.browsePath.isNotEmpty) {
                      favoritesNotifier.add(searchState.browsePath);
                    }
                  },
                  tooltip: 'Añadir carpeta actual',
                ),
              ],
            ),
          ),
          ...favorites.map((path) {
            final isSelected = searchState.browsePath == path;
            final name = path
                .split(RegExp(r'[\\/]'))
                .lastWhere((s) => s.isNotEmpty, orElse: () => path);
            return _aceptaSuelta(
              ListTile(
                dense: true,
                visualDensity: VisualDensity.compact,
                leading: const Icon(Icons.star, size: 16, color: Colors.amber),
                title: Text(
                  name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontSize: 12,
                    fontWeight:
                        isSelected ? FontWeight.bold : FontWeight.normal,
                    color:
                        isSelected ? AppColors.primary : AppColors.textPrimary,
                  ),
                ),
                trailing: IconButton(
                  icon: const Icon(Icons.close,
                      size: 14, color: AppColors.textSecondary),
                  onPressed: () => favoritesNotifier.remove(path),
                ),
                onTap: () => notifier.openFolder(path),
              ),
              path,
            );
          }),

          if (recentFolders.isNotEmpty) ...[
            const Divider(height: 16, color: AppColors.border),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
              child: Row(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
                children: [
                  const Text(
                    'CARPETAS RECIENTES',
                    style: TextStyle(
                      fontSize: 11,
                      fontWeight: FontWeight.bold,
                      color: AppColors.textSecondary,
                    ),
                  ),
                  IconButton(
                    icon: const Icon(Icons.clear_all,
                        size: 16, color: AppColors.textSecondary),
                    onPressed: () =>
                        ref.read(recentFoldersProvider.notifier).clear(),
                    tooltip: 'Limpiar recientes',
                  ),
                ],
              ),
            ),
            ...recentFolders.take(5).map((path) {
              final isSelected = searchState.browsePath == path ||
                  (searchState.browsePath.startsWith(path) && path.endsWith('\\'));
              final name = (path.endsWith(':\\') ||
                      path.endsWith(':/') ||
                      RegExp(r'^[a-zA-Z]:$').hasMatch(path))
                  ? 'Disco local (${path.replaceAll('\\', '').replaceAll('/', '')})'
                  : path
                      .split(RegExp(r'[\\/]'))
                      .lastWhere((s) => s.isNotEmpty, orElse: () => path);
              return _aceptaSuelta(
                ListTile(
                  dense: true,
                  visualDensity: VisualDensity.compact,
                  leading: const Icon(Icons.history_rounded,
                      size: 16, color: AppColors.textSecondary),
                  title: Text(
                    name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontSize: 12,
                      fontWeight:
                          isSelected ? FontWeight.bold : FontWeight.normal,
                      color: isSelected
                          ? AppColors.primary
                          : AppColors.textPrimary,
                    ),
                  ),
                  onTap: () => notifier.openFolder(path),
                ),
                path,
              );
            }),
          ],

          const Divider(height: 16, color: AppColors.border),

          // Encabezado de Volúmenes y Carpetas
          const Padding(
            padding: EdgeInsets.symmetric(horizontal: 12, vertical: 4),
            child: Text(
              'EQUIPO Y VOLÚMENES',
              style: TextStyle(
                fontSize: 11,
                fontWeight: FontWeight.bold,
                color: AppColors.textSecondary,
              ),
            ),
          ),

          if (serviceStatus != null)
            for (var i = 0; i < serviceStatus.volumePrefixes.length; i++)
              ..._raizVolumen(i, serviceStatus),
        ],
      ),
    ),
  );
}

  /// La rama raíz de un volumen: su propia fila y, al expandirla, el árbol de
  /// carpetas que cuelga de ella.
  List<Widget> _raizVolumen(int index, ffi.ServiceStatusFfi status) {
    final prefix = status.volumePrefixes[index];
    final label = index < status.volumeLabels.length
        ? status.volumeLabels[index]
        : prefix;
    final isConnected = index < status.volumeConnected.length
        ? status.volumeConnected[index]
        : true;
    final searchState = ref.read(searchProvider);
    final isSelected = searchState.browsePath == prefix;

    final expandida = _expandidas.contains(prefix);
    final raiz = _aceptaSuelta(
      InkWell(
        onTap: isConnected
            ? () => ref.read(searchProvider.notifier).openFolder(prefix)
            : null,
        child: Padding(
          padding: const EdgeInsets.only(left: 8, right: 4, top: 6, bottom: 6),
          child: Row(
            children: [
              InkWell(
                onTap: isConnected ? () => _alternarRama(prefix) : null,
                child: Icon(
                  expandida ? Icons.arrow_drop_down : Icons.arrow_right,
                  size: 18,
                  color: AppColors.textSecondary,
                ),
              ),
              Icon(
                isConnected ? Icons.dns : Icons.disc_full_outlined,
                size: 16,
                color: isConnected ? AppColors.primary : AppColors.textSecondary,
              ),
              const SizedBox(width: 6),
              Expanded(
                child: Text(
                  '$label ($prefix)',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontSize: 12,
                    fontWeight: isSelected ? FontWeight.bold : FontWeight.normal,
                    color: isSelected ? AppColors.primary : AppColors.textPrimary,
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
      prefix,
    );

    if (!expandida || !isConnected) return [raiz];
    return [
      raiz,
      Opacity(
        opacity: isConnected ? 1.0 : 0.4,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: _ramas(
            prefix,
            label,
            0,
          ),
        ),
      ),
    ];
  }

  List<_AccesoRapidoItem> _obtenerAccesosRapidos() {
    final env = Platform.environment;
    final home = Platform.isWindows ? env['USERPROFILE'] : env['HOME'];
    if (home == null || home.isEmpty) return const [];
    final sep = Platform.pathSeparator;
    return [
      _AccesoRapidoItem(
        nombre: 'Descargas',
        icono: Icons.download_rounded,
        color: const Color(0xFF29B6F6),
        ruta: '$home${sep}Downloads',
      ),
      _AccesoRapidoItem(
        nombre: 'Escritorio',
        icono: Icons.desktop_windows_rounded,
        color: const Color(0xFF66BB6A),
        ruta: '$home${sep}Desktop',
      ),
      _AccesoRapidoItem(
        nombre: 'Música',
        icono: Icons.library_music_rounded,
        color: const Color(0xFFFFA726),
        ruta: '$home${sep}Music',
      ),
      _AccesoRapidoItem(
        nombre: 'Documentos',
        icono: Icons.description_rounded,
        color: const Color(0xFFAB47BC),
        ruta: '$home${sep}Documents',
      ),
      _AccesoRapidoItem(
        nombre: 'Vídeos',
        icono: Icons.video_library_rounded,
        color: const Color(0xFFEC407A),
        ruta: '$home$sep${Platform.isWindows ? "Videos" : "Movies"}',
      ),
    ];
  }
}

class _AccesoRapidoItem {
  const _AccesoRapidoItem({
    required this.nombre,
    required this.icono,
    required this.color,
    required this.ruta,
  });

  final String nombre;
  final IconData icono;
  final Color color;
  final String ruta;
}
