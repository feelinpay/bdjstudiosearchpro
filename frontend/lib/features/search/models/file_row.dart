import 'dart:io';

import '../../../core/i18n/app_strings.dart';

String sanitizePath(String path) {
  var p = path.trim();
  if (Platform.isWindows && p.isNotEmpty) {
    final match = RegExp(r'^[\\/]+([a-zA-Z]:.*)$').firstMatch(p);
    if (match != null) {
      p = match.group(1)!;
    }
    if (RegExp(r'^[a-zA-Z]:$').hasMatch(p)) {
      p = '$p\\';
    }
  }
  return p;
}

/// Comprueba si una ruta corresponde a una unidad raíz o disco (C:\, D:\, /, /Volumes).
/// Estas rutas del sistema nunca se deben poder arrastrar ni mover.
bool esUnidadODisco(String ruta) {
  final r = ruta.trim();
  if (r.isEmpty) return true;
  if (Platform.isWindows) {
    return RegExp(r'^[a-zA-Z]:[\\/]?$').hasMatch(r);
  } else {
    return r == '/' || r == '/Volumes' || r == '/Volumes/';
  }
}

class FileRow {
  final int index;
  final String name;
  final String path;
  final String extension;
  final int size;
  final int mtime;
  final int flags;

  const FileRow({
    required this.index,
    required this.name,
    required this.path,
    required this.extension,
    required this.size,
    required this.mtime,
    required this.flags,
  });

  bool get isDirectory => (flags & 0x01) != 0;
  bool get isHidden => (flags & 0x02) != 0;
  bool get isDisconnected => (flags & 0x08) != 0;

  /// Etiqueta de tipo, idéntica a la del orden por «Tipo» del motor.
  String get tipoLabel {
    if (isDirectory) return 'Carpeta';
    switch (extension.toLowerCase()) {
      case 'wav':
      case 'flac':
      case 'aiff':
      case 'aif':
      case 'mp3':
      case 'm4a':
      case 'ogg':
      case 'opus':
      case 'wma':
      case 'alac':
      case 'ape':
      case 'wv':
      case 'aac':
        return 'Audio';
      case 'mp4':
      case 'mov':
      case 'avi':
      case 'mkv':
      case 'm4v':
      case 'webm':
      case 'mpg':
      case 'mpeg':
      case 'wmv':
      case 'flv':
        return 'Vídeo';
      case 'jpg':
      case 'jpeg':
      case 'png':
      case 'gif':
      case 'webp':
      case 'heic':
      case 'tiff':
      case 'bmp':
      case 'svg':
      case 'psd':
        return 'Imagen';
      case 'pdf':
      case 'docx':
      case 'doc':
      case 'txt':
      case 'rtf':
      case 'odt':
      case 'xlsx':
      case 'pptx':
      case 'md':
        return 'Documento';
      case 'als':
      case 'flp':
      case 'ptx':
      case 'cpr':
      case 'logicx':
      case 'rpp':
        return 'Proyecto';
      case 'zip':
      case 'rar':
      case '7z':
      case 'tar':
      case 'gz':
      case 'bz2':
        return 'Comprimido';
      case 'exe':
      case 'msi':
      case 'app':
      case 'dll':
      case 'dmg':
      case 'pkg':
        return 'Aplicación';
      case '':
        return 'Sin extensión';
      default:
        return 'Otro';
    }
  }

  String get fullPath {
    final cleanName = name.trim();
    final cleanPath = path.trim();
    if (cleanPath.isEmpty) return sanitizePath(cleanName);
    if (cleanName.isEmpty) return sanitizePath(cleanPath);
    if (cleanName == cleanPath) return sanitizePath(cleanName);

    var result = cleanName;
    if (Platform.isWindows && RegExp(r'^[a-zA-Z]:[\\/]').hasMatch(cleanName)) {
      result = cleanName;
    } else if (cleanPath.endsWith('\\') || cleanPath.endsWith('/')) {
      result = '$cleanPath$cleanName';
    } else {
      final sep = cleanPath.contains('/') ? '/' : '\\';
      result = '$cleanPath$sep$cleanName';
    }

    return sanitizePath(result);
  }

  String get formattedSize {
    if (isDirectory) return '';
    if (size == 0) return '0 B';
    const suffixes = ['B', 'KB', 'MB', 'GB', 'TB'];
    var s = size.toDouble();
    var i = 0;
    while (s >= 1024 && i < suffixes.length - 1) {
      s /= 1024;
      i++;
    }
    return '${s.toStringAsFixed(i == 0 ? 0 : 1)} ${suffixes[i]}';
  }

  static int _todayEpochDay = DateTime.now().millisecondsSinceEpoch ~/ 86400000;
  static int _lastDayCheck = 0;

  static int get _cachedTodayDay {
    final now = DateTime.now().millisecondsSinceEpoch;
    if (now - _lastDayCheck > 60000) {
      _todayEpochDay = now ~/ 86400000;
      _lastDayCheck = now;
    }
    return _todayEpochDay;
  }

  String get formattedDate {
    if (mtime == 0) return '';
    final dt = DateTime.fromMillisecondsSinceEpoch(mtime * 1000);
    final fileDay = (mtime * 1000) ~/ 86400000;
    final diffDays = _cachedTodayDay - fileDay;

    if (diffDays == 0) {
      final hour = dt.hour.toString().padLeft(2, '0');
      final min = dt.minute.toString().padLeft(2, '0');
      return '${AppStrings.today} $hour:$min';
    } else if (diffDays == 1) {
      final hour = dt.hour.toString().padLeft(2, '0');
      final min = dt.minute.toString().padLeft(2, '0');
      return '${AppStrings.yesterday} $hour:$min';
    } else {
      final y = dt.year;
      final m = dt.month.toString().padLeft(2, '0');
      final d = dt.day.toString().padLeft(2, '0');
      return '$y-$m-$d';
    }
  }
}
