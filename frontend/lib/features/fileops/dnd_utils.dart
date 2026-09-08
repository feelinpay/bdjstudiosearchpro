import 'dart:async';
import 'package:super_drag_and_drop/super_drag_and_drop.dart';

/// Formatos admitidos para arrastrar y soltar archivos entre ventanas o desde el explorador.
const kFileDropFormats = [Formats.fileUri, Formats.plainText];

/// Comprueba si la sesión de arrastre contiene archivos transferibles.
bool dropSessionTieneArchivos(DropSession session) {
  return session.items.any((item) =>
      item.localData != null ||
      item.canProvide(Formats.fileUri) ||
      item.canProvide(Formats.plainText));
}

/// Extrae las rutas de archivos de una sesión de arrastre (local o entre ventanas).
Future<List<String>> extraerRutasDeDropSession(DropSession session) async {
  final rutas = <String>[];
  for (final item in session.items) {
    final local = item.localData;
    if (local is String && local.trim().isNotEmpty) {
      rutas.addAll(
        local.split('\n').map((s) => s.trim()).where((s) => s.isNotEmpty),
      );
      continue;
    }
    final reader = item.dataReader;
    if (reader != null) {
      if (reader.canProvide(Formats.fileUri)) {
        final completer = Completer<Uri?>();
        final progress = reader.getValue<Uri>(Formats.fileUri, (val) {
          if (!completer.isCompleted) completer.complete(val);
        }, onError: (_) {
          if (!completer.isCompleted) completer.complete(null);
        });
        if (progress == null && !completer.isCompleted) {
          completer.complete(null);
        }
        final uri = await completer.future;
        if (uri != null && uri.isScheme('file')) {
          rutas.add(uri.toFilePath());
        }
      } else if (reader.canProvide(Formats.plainText)) {
        final completer = Completer<String?>();
        final progress = reader.getValue<String>(Formats.plainText, (val) {
          if (!completer.isCompleted) completer.complete(val);
        }, onError: (_) {
          if (!completer.isCompleted) completer.complete(null);
        });
        if (progress == null && !completer.isCompleted) {
          completer.complete(null);
        }
        final txt = await completer.future;
        if (txt != null && txt.trim().isNotEmpty) {
          rutas.addAll(
            txt.split('\n').map((s) => s.trim()).where((s) => s.isNotEmpty),
          );
        }
      }
    }
  }
  return rutas;
}
