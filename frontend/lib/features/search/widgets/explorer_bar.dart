import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../core/ffi/api.dart' as ffi;
import '../../../core/theme/app_colors.dart';
import '../providers/search_provider.dart';

/// Barra de navegación del explorador: atrás, adelante, subir, miga de pan y
/// una barra de ruta editable como la del Explorador de archivos.
///
/// **Está siempre.** Antes desaparecía salvo que estuvieras navegando una
/// carpeta, así que la aplicación cambiaba de forma según lo que estuvieras
/// haciendo: buscando parecía un cuadro de búsqueda y navegando parecía un
/// explorador. Es un explorador de archivos; la búsqueda rápida con filtros es
/// lo que lo distingue, no lo que lo define, y el marco no debe moverse por
/// escribir en una caja.
///
/// Lo único que cambia entre los dos modos es lo que dice la miga de pan: la
/// carpeta abierta, o dónde se está buscando.
///
/// El botón con el lápiz convierte la miga de pan en un campo donde se puede
/// escribir o pegar la ruta exacta; Enter navega, Esc vuelve a la miga.
/// Mientras se escribe se sugieren las carpetas que ya existen en disco (vía
/// `dart:io`, sin tocar el índice), igual en Windows y macOS.
class ExplorerBar extends ConsumerWidget {
  const ExplorerBar({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final estado = ref.watch(searchProvider);
    final notifier = ref.read(searchProvider.notifier);
    final buscando = estado.mode == ViewMode.search;

    return Container(
      height: 34,
      padding: const EdgeInsets.symmetric(horizontal: 8),
      decoration: const BoxDecoration(
        color: AppColors.surface,
        border: Border(bottom: BorderSide(color: AppColors.border)),
      ),
      child: Row(
        children: [
          _boton(
            icono: Icons.arrow_back_rounded,
            tooltip: 'Atrás',
            activo: estado.canGoBack,
            onTap: notifier.goBack,
          ),
          _boton(
            icono: Icons.arrow_forward_rounded,
            tooltip: 'Adelante',
            activo: estado.canGoForward,
            onTap: notifier.goForward,
          ),
          _boton(
            icono: Icons.arrow_upward_rounded,
            tooltip: 'Subir un nivel',
            activo: !buscando && estado.browsePath.isNotEmpty,
            onTap: notifier.goUp,
          ),
          const SizedBox(width: 8),
          const VerticalDivider(width: 1, color: AppColors.border, indent: 8, endIndent: 8),
          const SizedBox(width: 8),
          Expanded(
            child: buscando
                ? _AmbitoDeBusqueda(estado: estado)
                : _RutaEditable(path: estado.browsePath),
          ),
        ],
      ),
    );
  }

  Widget _boton({
    required IconData icono,
    required String tooltip,
    required bool activo,
    required VoidCallback onTap,
  }) {
    return IconButton(
      icon: Icon(icono, size: 17),
      tooltip: tooltip,
      color: activo ? AppColors.textSecondary : AppColors.textDisabled,
      onPressed: activo ? onTap : null,
      splashRadius: 16,
      padding: EdgeInsets.zero,
      constraints: const BoxConstraints(minWidth: 30, minHeight: 30),
    );
  }
}

/// Dónde se está buscando, y el interruptor para ampliar a todo el equipo.
///
/// Ocupa el sitio de la miga de pan mientras hay una búsqueda en marcha, para
/// que la barra no cambie de altura ni de forma: el usuario siempre sabe dónde
/// está mirando, esté navegando o filtrando.
class _AmbitoDeBusqueda extends ConsumerWidget {
  const _AmbitoDeBusqueda({required this.estado});

  final SearchState estado;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final notifier = ref.read(searchProvider.notifier);
    final acotado = !estado.searchEverywhere && estado.searchScope.isNotEmpty;
    final donde = acotado ? estado.searchScope : 'Este equipo';

    return Row(
      children: [
        const Icon(Icons.search_rounded, size: 15, color: AppColors.textSecondary),
        const SizedBox(width: 6),
        Flexible(
          child: Text(
            'Resultados en $donde',
            overflow: TextOverflow.ellipsis,
            style: const TextStyle(fontSize: 12, color: AppColors.textSecondary),
          ),
        ),
        const SizedBox(width: 10),
        if (estado.searchScope.isNotEmpty)
          TextButton.icon(
            onPressed: notifier.toggleSearchEverywhere,
            icon: Icon(
              acotado ? Icons.travel_explore_rounded : Icons.folder_open_rounded,
              size: 14,
            ),
            label: Text(
              acotado ? 'Buscar en todo el equipo' : 'Buscar solo en esta carpeta',
              style: const TextStyle(fontSize: 11),
            ),
            style: TextButton.styleFrom(
              foregroundColor: AppColors.primary,
              padding: const EdgeInsets.symmetric(horizontal: 8),
              minimumSize: const Size(0, 26),
              tapTargetSize: MaterialTapTargetSize.shrinkWrap,
            ),
          ),
        const Spacer(),
        TextButton.icon(
          onPressed: notifier.exitSearch,
          icon: const Icon(Icons.close_rounded, size: 14),
          label: const Text('Salir de la búsqueda', style: TextStyle(fontSize: 11)),
          style: TextButton.styleFrom(
            foregroundColor: AppColors.textSecondary,
            padding: const EdgeInsets.symmetric(horizontal: 8),
            minimumSize: const Size(0, 26),
            tapTargetSize: MaterialTapTargetSize.shrinkWrap,
          ),
        ),
      ],
    );
  }
}

/// Muestra la miga de pan o, si se pulsa el lápiz, un campo con la ruta exacta
/// editable y con autocompletado de carpetas existentes.
class _RutaEditable extends ConsumerStatefulWidget {
  const _RutaEditable({required this.path});

  final String path;

  @override
  ConsumerState<_RutaEditable> createState() => _RutaEditableState();
}

class _RutaEditableState extends ConsumerState<_RutaEditable> {
  bool _editando = false;
  final List<String> _sugerencias = <String>[];
  Timer? _debounce;
  int _seqSug = 0;
  TextEditingController? _controlador;
  FocusNode? _nodoEnfoque;

  @override
  void dispose() {
    _debounce?.cancel();
    _controlador?.dispose();
    _nodoEnfoque?.dispose();
    super.dispose();
  }

  Future<bool> _navegar(String ruta) async {
    final destino = _normalizar(_expandirTilde(ruta.trim()));
    if (destino.isEmpty) {
      setState(() => _editando = false);
      return true;
    }
    // Si la ruta apunta a un archivo, ir a su carpeta (como el Explorador).
    var ir = destino;
    try {
      final tipo = FileSystemEntity.typeSync(destino);
      if (tipo == FileSystemEntityType.file) {
        ir = File(destino).parent.path;
      }
    } catch (_) {}
    final ok = await ref.read(searchProvider.notifier).openFolder(ir);
    if (!mounted) return ok;
    setState(() => _editando = false);
    if (!ok) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('No se pudo abrir: $destino')),
      );
    }
    return ok;
  }

  void _empezarEdicion() {
    _controlador ??= TextEditingController(text: widget.path);
    _nodoEnfoque ??= FocusNode();
    setState(() {
      _editando = true;
      _sugerencias.clear();
    });
  }

  void _salirEdicion() {
    _debounce?.cancel();
    setState(() {
      _editando = false;
      _sugerencias.clear();
    });
  }

  Future<void> _actualizarSugerencias(String texto) async {
    _debounce?.cancel();
    final seq = ++_seqSug;
    _debounce = Timer(const Duration(milliseconds: 120), () async {
      final sugs = await _sugerir(texto);
      if (!mounted || seq != _seqSug) return;
      setState(() {
        _sugerencias
          ..clear()
          ..addAll(sugs);
      });
    });
  }

  /// Sugerencias en base a lo que se está escribiendo: lista las carpetas que
  /// ya existen en disco bajo el directorio más profundo que coincida.
  Future<List<String>> _sugerir(String texto) async {
    final sep = Platform.pathSeparator;
    var t = _expandirTilde(_normalizar(texto));
    if (t.isEmpty) return _raices();

    var candidato = t.endsWith(sep) ? t.substring(0, t.length - 1) : t;
    if (Platform.isWindows && RegExp(r'^[A-Za-z]:$').hasMatch(candidato)) {
      candidato = '$candidato\\';
    }

    // Directorio existente más profundo que contiene lo escrito hasta ahora.
    var base = '';
    final vistos = <String>{};
    while (vistos.add(candidato)) {
      if (candidato.isNotEmpty && Directory(candidato).existsSync()) {
        base = candidato;
        break;
      }
      final padre = Directory(candidato.isEmpty ? '.' : candidato).parent.path;
      if (padre.isEmpty || padre == candidato) break;
      candidato = padre;
    }

    final lista = base;
    if (lista.isEmpty) {
      // Sin directorio existente: mostrar las raíces y dejar que el usuario
      // siga escribiendo.
      return _raices();
    }

    final tic = t.toLowerCase();
    final hijos = <String>[];
    try {
      for (final e in Directory(lista).listSync(followLinks: false)) {
        if (e is Directory) {
          final nombre = e.path;
          if (nombre.toLowerCase().startsWith(tic)) {
            hijos.add(nombre);
          }
          if (hijos.length >= 40) break;
        }
      }
    } catch (_) {}

    // Añade además el propio texto cuando ya es una carpeta existente, para
    // poder pulsar Enter y quedarse aquí.
    if (Directory(lista).existsSync() && lista.toLowerCase().startsWith(tic)) {
      hijos.insert(0, lista);
    }
    return hijos;
  }

  /// Raíces de unidades y accesos rápidos mostrados al empezar a escribir.
  Future<List<String>> _raices() async {
    final raices = <String>[];
    if (Platform.isWindows) {
      // Sin `Directory.listRoots` (no existe en este SDK): probar cada letra.
      for (var c = 65; c <= 90; c++) {
        final ruta = '${String.fromCharCode(c)}:\\';
        try {
          if (Directory(ruta).existsSync()) raices.add(ruta);
        } catch (_) {}
      }
      return raices;
    }
    // macOS/Linux.
    raices.add('/');
    raices.add(_home);
    for (final d in const ['Desktop', 'Documents', 'Downloads', 'Music', 'Pictures', 'Movies']) {
      final p = '$_home/$d';
      if (Directory(p).existsSync()) raices.add(p);
    }
    const vol = '/Volumes';
    if (Directory(vol).existsSync()) raices.add(vol);
    return raices;
  }

  /// Expande `~` (y `~/...`) a la carpeta personal, en cualquier sistema.
  String _expandirTilde(String r) {
    if (r.isEmpty) return r;
    if (r == '~') return _home;
    if (r.startsWith('~/') || r.startsWith('~\\')) return _home + r.substring(1);
    return r;
  }

  String get _home {
    final env = Platform.environment;
    final h = Platform.isWindows ? env['USERPROFILE'] : env['HOME'];
    if (h != null && h.isNotEmpty) return h;
    return Platform.isWindows ? 'C:\\Users\\' : '/';
  }

  /// Unifica los separadores al del sistema y quita los finales (salvo raíz).
  String _normalizar(String r) {
    var t = r.replaceAll(Platform.isWindows ? '/' : '\\', Platform.pathSeparator);
    if (t.isEmpty) return '';
    final esRaiz =
        t == Platform.pathSeparator || RegExp(r'^[A-Za-z]:\\$').hasMatch(t);
    if (!esRaiz) {
      while (t.length > 1 && t.endsWith(Platform.pathSeparator)) {
        t = t.substring(0, t.length - 1);
      }
    }
    return t;
  }

  @override
  Widget build(BuildContext context) {
    if (!_editando) {
      return Row(
        children: [
          Expanded(child: _MigaDePan(path: widget.path)),
          IconButton(
            icon: const Icon(Icons.edit_outlined, size: 15),
            tooltip: 'Escribir la ruta exacta',
            color: AppColors.textSecondary,
            onPressed: _empezarEdicion,
            splashRadius: 16,
            padding: EdgeInsets.zero,
            constraints: const BoxConstraints(minWidth: 28, minHeight: 28),
          ),
        ],
      );
    }

    return RawAutocomplete<String>(
      textEditingController: _controlador!,
      focusNode: _nodoEnfoque!,
      optionsBuilder: (TextEditingValue te) {
        final tic = te.text.trim().toLowerCase();
        return _sugerencias.where((s) => s.toLowerCase().startsWith(tic));
      },
      displayStringForOption: (s) => s,
      onSelected: _navegar,
      fieldViewBuilder: (context, controller, focusNode, onFieldSubmitted) {
        return Focus(
          onKeyEvent: (node, event) {
            if (event is KeyDownEvent &&
                event.logicalKey == LogicalKeyboardKey.escape) {
              _salirEdicion();
              return KeyEventResult.handled;
            }
            return KeyEventResult.ignored;
          },
          child: TextField(
            controller: controller,
            focusNode: focusNode,
            autofocus: true,
            textInputAction: TextInputAction.go,
            style: const TextStyle(fontSize: 12, color: AppColors.textPrimary),
            decoration: InputDecoration(
              isDense: true,
              hintText: 'Ruta exacta (Enter para ir, Esc para salir)',
              hintStyle: const TextStyle(fontSize: 12, color: AppColors.textDisabled),
              contentPadding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
              filled: true,
              fillColor: AppColors.surfaceAlt,
              border: OutlineInputBorder(
                borderRadius: BorderRadius.circular(4),
                borderSide: const BorderSide(color: AppColors.border),
              ),
              enabledBorder: OutlineInputBorder(
                borderRadius: BorderRadius.circular(4),
                borderSide: const BorderSide(color: AppColors.border),
              ),
              focusedBorder: OutlineInputBorder(
                borderRadius: BorderRadius.circular(4),
                borderSide: const BorderSide(color: AppColors.primary),
              ),
            ),
            onChanged: _actualizarSugerencias,
            onSubmitted: (v) {
              onFieldSubmitted();
              _navegar(v);
            },
          ),
        );
      },
      optionsViewBuilder: (context, onSelected, options) {
        return Align(
          alignment: Alignment.topLeft,
          child: Material(
            elevation: 6,
            color: AppColors.surfaceAlt,
            borderRadius: const BorderRadius.vertical(bottom: Radius.circular(6)),
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxHeight: 240),
              child: ListView.builder(
                shrinkWrap: true,
                padding: EdgeInsets.zero,
                itemCount: options.length,
                itemBuilder: (context, i) {
                  final o = options.elementAt(i);
                  return ListTile(
                    dense: true,
                    leading: const Icon(Icons.folder_outlined,
                        size: 16, color: AppColors.textSecondary),
                    title: Text(o, style: const TextStyle(fontSize: 12)),
                    onTap: () => onSelected(o),
                  );
                },
              ),
            ),
          ),
        );
      },
    );
  }
}

/// Los tramos de la ruta, cada uno pulsable.
class _MigaDePan extends ConsumerWidget {
  const _MigaDePan({required this.path});

  final String path;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final notifier = ref.read(searchProvider.notifier);

    return FutureBuilder<List<ffi.CrumbFfi>>(
      // La miga se calcula en el motor, que es quien sabe separar «C:\» de una
      // ruta de estilo Unix.
      future: notifier.breadcrumbOf(path),
      builder: (context, snapshot) {
        final tramos = snapshot.data ?? const <ffi.CrumbFfi>[];
        return SingleChildScrollView(
          scrollDirection: Axis.horizontal,
          reverse: true,
          child: Row(
            children: [
              InkWell(
                onTap: () => notifier.openFolder(''),
                child: const Padding(
                  padding: EdgeInsets.symmetric(horizontal: 6, vertical: 4),
                  child: Icon(Icons.storage_rounded, size: 15, color: AppColors.textSecondary),
                ),
              ),
              for (var i = 0; i < tramos.length; i++) ...[
                const Icon(Icons.chevron_right_rounded,
                    size: 15, color: AppColors.textDisabled),
                InkWell(
                  onTap: () => notifier.openFolder(tramos[i].path),
                  child: Padding(
                    padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 4),
                    child: Text(
                      tramos[i].name,
                      style: TextStyle(
                        fontSize: 12,
                        fontWeight: i == tramos.length - 1
                            ? FontWeight.w600
                            : FontWeight.w400,
                        color: i == tramos.length - 1
                            ? AppColors.textPrimary
                            : AppColors.textSecondary,
                      ),
                    ),
                  ),
                ),
              ],
            ],
          ),
        );
      },
    );
  }
}