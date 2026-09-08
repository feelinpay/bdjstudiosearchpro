import 'dart:async';
import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:super_drag_and_drop/super_drag_and_drop.dart';
import '../../../core/i18n/app_strings.dart';
import '../../../core/theme/app_colors.dart';
import '../../../core/ffi/api.dart' as ffi;
import '../../fileops/dnd_utils.dart';
import '../../fileops/providers/file_ops_provider.dart';
import '../models/file_row.dart';
import '../providers/favorites_provider.dart';
import '../providers/search_provider.dart';

class _VolumenItem {
  final String prefix;
  final String label;
  final bool isConnected;
  final bool isRemovable;

  const _VolumenItem({
    required this.prefix,
    required this.label,
    required this.isConnected,
    required this.isRemovable,
  });
}

/// Panel lateral de navegación con Marcadores (favoritos) y equipo/volúmenes.
class SidebarTree extends ConsumerStatefulWidget {
  const SidebarTree({super.key});

  @override
  ConsumerState<SidebarTree> createState() => _SidebarTreeState();
}

class _SidebarTreeState extends ConsumerState<SidebarTree> {
  ffi.ServiceStatusFfi? _serviceStatus;
  Timer? _pollTimer;

  /// Subdirectorios conocidos de cada carpeta rama del árbol (carga perezosa).
  final Map<String, List<String>> _subdirs = {};

  /// Carpetas cuya rama está desplegada en el árbol.
  final Set<String> _expandidas = {};

  @override
  void initState() {
    super.initState();
    _loadServiceStatus();
    _pollTimer = Timer.periodic(const Duration(seconds: 3), (_) {
      if (mounted) {
        _loadServiceStatus();
      }
    });
  }

  @override
  void dispose() {
    _pollTimer?.cancel();
    super.dispose();
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
    final filtradas = rutas.where((r) => !esUnidadODisco(r)).toList();
    if (filtradas.isEmpty) return;

    final ops = ref.read(fileOpsProvider.notifier);
    await ops.moveTo(filtradas, destino);
    await ref.read(searchProvider.notifier).refreshAfterFileOperation();
  }

  /// Envuelve una carpeta del árbol lateral para aceptar sueltas (mover).
  Widget _aceptaSuelta(Widget child, String destino) {
    return DropRegion(
      formats: kFileDropFormats,
      onDropOver: (event) async {
        if (dropSessionTieneArchivos(event.session)) {
          return DropOperation.move;
        }
        return DropOperation.none;
      },
      onPerformDrop: (event) async {
        final rutas = await extraerRutasDeDropSession(event.session);
        if (rutas.isNotEmpty) {
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
        if (hijos.isEmpty && Directory(path).existsSync()) {
          hijos = Directory(path)
              .listSync(followLinks: false)
              .whereType<Directory>()
              .map((d) => d.path)
              .toList()
            ..sort((a, b) => a.toLowerCase().compareTo(b.toLowerCase()));
        }
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
    final browsePath = ref.watch(searchProvider.select((s) => s.browsePath));
    final notifier = ref.read(searchProvider.notifier);
    final favorites = ref.watch(favoritesProvider);
    final favoritesNotifier = ref.read(favoritesProvider.notifier);

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
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
            child: Text(
              AppStrings.quickAccess,
              style: const TextStyle(
                fontSize: 11,
                fontWeight: FontWeight.bold,
                color: AppColors.textSecondary,
              ),
            ),
          ),
          ..._obtenerAccesosRapidos().map((item) {
            final isSelected = browsePath == item.ruta ||
                (browsePath.startsWith(item.ruta) &&
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
                Expanded(
                  child: Text(
                    AppStrings.myBookmarks,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(
                      fontSize: 11,
                      fontWeight: FontWeight.bold,
                      color: AppColors.textSecondary,
                    ),
                  ),
                ),
                IconButton(
                  padding: const EdgeInsets.all(4),
                  constraints: const BoxConstraints(),
                  icon: const Icon(Icons.add,
                      size: 16, color: AppColors.textSecondary),
                  onPressed: () {
                    if (browsePath.isNotEmpty) {
                      favoritesNotifier.add(browsePath);
                    }
                  },
                  tooltip: 'Añadir carpeta actual',
                ),
              ],
            ),
          ),
          ...favorites.map((path) {
            final isSelected = browsePath == path;
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

          const Divider(height: 16, color: AppColors.border),

          // Encabezado de Volúmenes y Carpetas
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
            child: Text(
              AppStrings.drivesAndVolumes,
              style: const TextStyle(
                fontSize: 11,
                fontWeight: FontWeight.bold,
                color: AppColors.textSecondary,
              ),
            ),
          ),

          for (final vol in _obtenerVolumenes())
            ..._raizVolumen(vol),
        ],
      ),
    ),
  );
}

  List<_VolumenItem> _obtenerVolumenes() {
    final Map<String, _VolumenItem> map = {};

    // 1. Volúmenes reportados por el motor de indexación
    if (_serviceStatus != null) {
      for (var i = 0; i < _serviceStatus!.volumePrefixes.length; i++) {
        final prefix = _serviceStatus!.volumePrefixes[i];
        final label = i < _serviceStatus!.volumeLabels.length ? _serviceStatus!.volumeLabels[i] : prefix;
        final isConnected = i < _serviceStatus!.volumeConnected.length ? _serviceStatus!.volumeConnected[i] : true;
        map[prefix.toLowerCase()] = _VolumenItem(
          prefix: prefix,
          label: label.isEmpty ? prefix : label,
          isConnected: isConnected,
          isRemovable: false,
        );
      }
    }

    // 2. Detección directa en caliente en el SO (USBs, discos extraíbles, etc.)
    if (Platform.isWindows) {
      for (var c = 65; c <= 90; c++) {
        final letter = String.fromCharCode(c);
        final drivePath = '$letter:\\';
        try {
          if (Directory(drivePath).existsSync()) {
            final key = drivePath.toLowerCase();
            final yaEsta = map.containsKey(key);
            final esRemovible = c > 67; // D: en adelante suele ser secundario/USB
            if (!yaEsta) {
              map[key] = _VolumenItem(
                prefix: drivePath,
                label: 'Unidad $letter:',
                isConnected: true,
                isRemovable: esRemovible,
              );
            } else if (esRemovible) {
              // Marcar como extraíble para mostrar el icono USB
              map[key] = _VolumenItem(
                prefix: map[key]!.prefix,
                label: map[key]!.label,
                isConnected: map[key]!.isConnected,
                isRemovable: true,
              );
            }
          }
        } catch (_) {}
      }
    } else if (Platform.isMacOS) {
      if (!map.containsKey('/')) {
        map['/'] = const _VolumenItem(
          prefix: '/',
          label: 'Macintosh HD',
          isConnected: true,
          isRemovable: false,
        );
      }
      try {
        final volumesDir = Directory('/Volumes');
        if (volumesDir.existsSync()) {
          for (final entity in volumesDir.listSync()) {
            if (entity is Directory) {
              final path = entity.path;
              final name = path.split('/').last;
              if (name.isNotEmpty && !name.startsWith('.')) {
                map[path.toLowerCase()] = _VolumenItem(
                  prefix: path,
                  label: name,
                  isConnected: true,
                  isRemovable: true,
                );
              }
            }
          }
        }
      } catch (_) {}
    }

    return map.values.toList();
  }

  /// La rama raíz de un volumen: su propia fila y, al expandirla, el árbol de
  /// carpetas que cuelga de ella.
  List<Widget> _raizVolumen(_VolumenItem item) {
    final prefix = item.prefix;
    final label = item.label;
    final isConnected = item.isConnected;
    final isRemovable = item.isRemovable;
    final browsePath = ref.watch(searchProvider.select((s) => s.browsePath));
    final isSelected = browsePath == prefix;

    final expandida = _expandidas.contains(prefix);
    final icono = isRemovable
        ? Icons.usb_rounded
        : (isConnected ? Icons.dns_rounded : Icons.disc_full_outlined);

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
                icono,
                size: 16,
                color: isConnected
                    ? (isRemovable ? const Color(0xFF00B4D8) : AppColors.primary)
                    : AppColors.textSecondary,
              ),
              const SizedBox(width: 6),
              Expanded(
                child: Text(
                  label == prefix ? prefix : '$label ($prefix)',
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
