import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../search/providers/search_provider.dart';
import 'providers/file_ops_provider.dart';

/// Confirmación de envío a la papelera (reversible).
///
/// Es lo que usa tanto el menú como el atajo `Supr`: da un punto único de
/// coherencia entre las dos vías.
Future<void> confirmarEnviarALaPapelera(
  BuildContext context,
  FileOpsNotifier ops,
  int cuantos,
) async {
  final confirmado = await showDialog<bool>(
    context: context,
    builder: (ctx) => AlertDialog(
      title: const Text('Enviar a la papelera'),
      content: Text(
        cuantos == 1
            ? '¿Enviar el elemento seleccionado a la papelera?'
            : '¿Enviar $cuantos elementos a la papelera?',
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(ctx).pop(false),
          child: const Text('Cancelar'),
        ),
        TextButton(
          onPressed: () => Navigator.of(ctx).pop(true),
          child: const Text('Enviar a la papelera'),
        ),
      ],
    ),
  );
  if (confirmado != true) return;
  await ops.trashSelection();
}

/// Eliminación definitiva: irreversible.
///
/// Se pide escribir la palabra «ELIMINAR» en el diálogo porque no hay vuelta
/// atrás: un DJ que pierde una sesión pierde trabajo de meses. Un clic solo no
/// basta para algo que se sale de la papelera.
Future<void> eliminarPermanentemente(
  BuildContext context,
  FileOpsNotifier ops,
  int cuantos,
) async {
  final confirmado = await showDialog<bool>(
    context: context,
    builder: (ctx) => _DialogoEliminar(cuantos: cuantos),
  );
  if (confirmado != true) return;
  await ops.deletePermanentlySelection();
}

/// Diálogo que exige escribir «ELIMINAR».
///
/// Es un StatefulWidget que **posee** el `TextEditingController`: lo crea en su
/// `initState` y lo suelta en su `dispose`. Si el controller se crea fuera y se
/// llama a `dispose()` al devolver `showDialog`, la animación de salida del
/// diálogo sigue un par de fotogramas más y Flutter se queja de un controller
/// usado tras ser liberado («Nueva carpeta» lo provocaba: crash garantizado).
class _DialogoEliminar extends StatefulWidget {
  const _DialogoEliminar({required this.cuantos});

  final int cuantos;

  @override
  State<_DialogoEliminar> createState() => _DialogoEliminarState();
}

class _DialogoEliminarState extends State<_DialogoEliminar> {
  final _controlador = TextEditingController();

  @override
  void dispose() {
    _controlador.dispose();
    super.dispose();
  }

  void _confirmarSiProcede(String v) {
    if (v.trim().toUpperCase() == 'ELIMINAR') {
      Navigator.of(context).pop(true);
    }
  }

  @override
  Widget build(BuildContext context) {
    final listo = _controlador.text.trim().toUpperCase() == 'ELIMINAR';
    return AlertDialog(
      title: const Text('Eliminar permanentemente'),
      content: SizedBox(
        width: 380,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              widget.cuantos == 1
                  ? 'El elemento seleccionado se borrará del disco sin '
                      'pasar por la papelera. Esta acción no se puede '
                      'deshacer.'
                  : '${widget.cuantos} elementos se borrarán del disco sin '
                      'pasar por la papelera. Esta acción no se puede '
                      'deshacer.',
              style: const TextStyle(fontSize: 13),
            ),
            const SizedBox(height: 14),
            TextField(
              controller: _controlador,
              autofocus: true,
              onChanged: (_) => setState(() {}),
              decoration: const InputDecoration(
                labelText: 'Escribe «ELIMINAR» para confirmar',
                hintText: 'ELIMINAR',
              ),
              onSubmitted: _confirmarSiProcede,
            ),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(false),
          child: const Text('Cancelar'),
        ),
        TextButton(
          style: TextButton.styleFrom(foregroundColor: Colors.red),
          onPressed: listo ? () => Navigator.of(context).pop(true) : null,
          child: const Text('Eliminar para siempre'),
        ),
      ],
    );
  }
}

// ───────────────────── Diálogos de nombre (compartidos) ─────────────────────

/// Pide un nombre de archivo o carpeta.
///
/// El texto llega preseleccionado sin la extensión: es lo que se cambia casi
/// siempre, y así no hay que borrarla a mano.
Future<String?> pedirTexto(
  BuildContext context, {
  required String titulo,
  required String etiqueta,
  required String inicial,
}) {
  // El diálogo es un StatefulWidget que posee el controller; aquí solo se le
  // pasan los parámetros. Crear el controller en esta función y liberarlo con
  // `.whenComplete` era el bug: la animación de cierre sigue vivo un par de
  // fotogramas y el campo vuelve a usarse tras el `dispose()`.
  return showDialog<String>(
    context: context,
    builder: (ctx) => _DialogoNombre(
      titulo: titulo,
      etiqueta: etiqueta,
      inicial: inicial,
    ),
  );
}

/// Diálogo de nombre (nueva carpeta, nuevo archivo, cambiar nombre).
///
/// Posee el `TextEditingController` (creado en `initState`, liberado en
/// `dispose`) para que ninguna animación de salida lo vuelva a tocar después
/// de haberse liberado.
class _DialogoNombre extends StatefulWidget {
  const _DialogoNombre({
    required this.titulo,
    required this.etiqueta,
    required this.inicial,
  });

  final String titulo;
  final String etiqueta;
  final String inicial;

  @override
  State<_DialogoNombre> createState() => _DialogoNombreState();
}

class _DialogoNombreState extends State<_DialogoNombre> {
  late final TextEditingController _controlador;

  @override
  void initState() {
    super.initState();
    _controlador = TextEditingController(text: widget.inicial);
    final punto = widget.inicial.lastIndexOf('.');
    _controlador.selection = TextSelection(
      baseOffset: 0,
      extentOffset: punto > 0 ? punto : widget.inicial.length,
    );
  }

  @override
  void dispose() {
    _controlador.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Text(widget.titulo),
      content: TextField(
        controller: _controlador,
        autofocus: true,
        decoration: InputDecoration(labelText: widget.etiqueta),
        onSubmitted: (v) => Navigator.of(context).pop(v),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancelar'),
        ),
        TextButton(
          onPressed: () => Navigator.of(context).pop(_controlador.text),
          child: const Text('Aceptar'),
        ),
      ],
    );
  }
}

/// Crea una carpeta nueva en `destino`, previa pregunta del nombre.
Future<void> nuevaCarpetaDialog(
  BuildContext context,
  FileOpsNotifier ops,
  String destino,
) async {
  final nombre = await pedirTexto(
    context,
    titulo: 'Nueva carpeta',
    etiqueta: 'Nombre de la carpeta',
    inicial: 'Nueva carpeta',
  );
  if (nombre == null || nombre.trim().isEmpty) return;
  await ops.createFolder(destino, nombre.trim());
}

/// Crea un archivo nuevo en `destino`, previa pregunta del nombre.
///
/// El nombre por defecto se numera automáticamente si ya existe, como hace el
/// Explorador: «Nuevo documento de texto.txt», «Nuevo documento de texto (2)…»
Future<void> nuevoArchivoDialog(
  BuildContext context,
  FileOpsNotifier ops,
  String destino,
) async {
  final inicial = _nombreLibre(
    destino,
    'Nuevo documento de texto.txt',
  );
  final nombre = await pedirTexto(
    context,
    titulo: 'Nuevo archivo',
    etiqueta: 'Nombre del archivo',
    inicial: inicial,
  );
  if (nombre == null || nombre.trim().isEmpty) return;
  await ops.createFile(destino, nombre.trim());
}

/// Renombra lo seleccionado (la primera ruta), previa pregunta del nombre.
Future<void> renombrarDialog(
  BuildContext context,
  WidgetRef ref,
  FileOpsNotifier ops,
) async {
  final rutas = await ref.read(searchProvider.notifier).selectedPaths(max: 1);
  if (rutas.isEmpty) return;
  final ruta = rutas.first;
  final actual = ruta.split(RegExp(r'[\\/]')).last;

  if (!context.mounted) return;
  final nombre = await pedirTexto(
    context,
    titulo: 'Cambiar nombre',
    etiqueta: 'Nombre nuevo',
    inicial: actual,
  );
  if (nombre == null || nombre.trim().isEmpty || nombre.trim() == actual) return;
  await ops.rename(ruta, nombre.trim());
}

/// Un nombre que no colisiona con nada que ya esté en disco.
String _nombreLibre(String padre, String base) {
  final sep = Platform.pathSeparator;
  final punto = base.lastIndexOf('.');
  final sinExt = punto > 0 ? base.substring(0, punto) : base;
  final ext = punto > 0 ? base.substring(punto) : '';

  bool ocupa(String nombre) {
    final ruta = padre.isEmpty || padre.endsWith(sep)
        ? '$padre$nombre'
        : '$padre$sep$nombre';
    try {
      return File(ruta).existsSync() || Directory(ruta).existsSync();
    } catch (_) {
      return false;
    }
  }

  var nombre = base;
  var i = 2;
  while (ocupa(nombre)) {
    nombre = '$sinExt ($i)$ext';
    i++;
  }
  return nombre;
}