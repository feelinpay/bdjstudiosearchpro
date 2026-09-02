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
    if (path.endsWith('\\') || path.endsWith('/')) {
      return '$path$name';
    }
    final sep = path.contains('/') ? '/' : '\\';
    return '$path$sep$name';
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

  String get formattedDate {
    if (mtime == 0) return '';
    final dt = DateTime.fromMillisecondsSinceEpoch(mtime * 1000);
    final now = DateTime.now();
    final diff = now.difference(dt);

    if (diff.inDays == 0) {
      final hour = dt.hour.toString().padLeft(2, '0');
      final min = dt.minute.toString().padLeft(2, '0');
      return 'Hoy $hour:$min';
    } else if (diff.inDays == 1) {
      final hour = dt.hour.toString().padLeft(2, '0');
      final min = dt.minute.toString().padLeft(2, '0');
      return 'Ayer $hour:$min';
    } else {
      final y = dt.year;
      final m = dt.month.toString().padLeft(2, '0');
      final d = dt.day.toString().padLeft(2, '0');
      return '$y-$m-$d';
    }
  }
}
