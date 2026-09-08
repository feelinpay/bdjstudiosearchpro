import 'dart:io';

/// Textos de la aplicacion adaptados automaticamente al idioma del sistema.
///
/// Detecta el locale de Windows o macOS (ej: 'es_ES', 'en_US') sin dependencias
/// externas y con acceso O(1) directo para velocidad maxima.
abstract class AppStrings {
  static final bool isSpanish = Platform.localeName.toLowerCase().startsWith('es');

  // Menus principales
  static String get file => isSpanish ? 'Archivo' : 'File';
  static String get edit => isSpanish ? 'Edición' : 'Edit';
  static String get view => isSpanish ? 'Ver' : 'View';
  static String get search => isSpanish ? 'Búsqueda' : 'Search';
  static String get bookmarks => isSpanish ? 'Marcadores' : 'Bookmarks';

  static String get newWindow => isSpanish ? 'Nueva ventana' : 'New window';
  static String get closeWindow => isSpanish ? 'Cerrar ventana' : 'Close window';
  static String get exit => isSpanish ? 'Salir' : 'Exit';

  static String get cut => isSpanish ? 'Cortar' : 'Cut';
  static String get copy => isSpanish ? 'Copiar' : 'Copy';
  static String get paste => isSpanish ? 'Pegar' : 'Paste';
  static String get duplicate => isSpanish ? 'Duplicar' : 'Duplicate';
  static String get rename => isSpanish ? 'Cambiar nombre' : 'Rename';
  static String get delete => isSpanish ? 'Enviar a la papelera' : 'Send to Trash';
  static String get deleteForever => isSpanish ? 'Eliminar permanentemente…' : 'Delete permanently…';
  static String get selectAll => isSpanish ? 'Seleccionar todo' : 'Select all';
  static String get invertSelection => isSpanish ? 'Invertir la selección' : 'Invert selection';

  // Vistas
  static String get details => isSpanish ? 'Detalles' : 'Details';
  static String get list => isSpanish ? 'Lista' : 'List';
  static String get compact => isSpanish ? 'Compacta' : 'Compact';
  static String get icons => isSpanish ? 'Iconos grandes' : 'Large icons';

  // Columnas de la tabla
  static String get name => isSpanish ? 'Nombre' : 'Name';
  static String get path => isSpanish ? 'Ruta' : 'Path';
  static String get type => isSpanish ? 'Tipo' : 'Type';
  static String get extension => isSpanish ? 'Ext' : 'Ext';
  static String get size => isSpanish ? 'Tamaño' : 'Size';
  static String get dateModified => isSpanish ? 'Modificado' : 'Date modified';

  // Barra lateral
  static String get quickAccess => isSpanish ? 'ACCESO RÁPIDO' : 'QUICK ACCESS';
  static String get myBookmarks => isSpanish ? 'MIS MARCADORES' : 'MY BOOKMARKS';
  static String get drivesAndVolumes => isSpanish ? 'EQUIPO Y VOLÚMENES' : 'DRIVES & VOLUMES';
  static String get downloads => isSpanish ? 'Descargas' : 'Downloads';
  static String get desktop => isSpanish ? 'Escritorio' : 'Desktop';
  static String get music => isSpanish ? 'Música' : 'Music';
  static String get documents => isSpanish ? 'Documentos' : 'Documents';
  static String get videos => isSpanish ? 'Vídeos' : 'Videos';

  // Filtros rapidos
  static String get all => isSpanish ? 'Todos' : 'All';
  static String get audio => 'Audio';
  static String get djProjects => isSpanish ? 'Proyectos DJ' : 'DJ Projects';
  static String get video => isSpanish ? 'Vídeo' : 'Video';
  static String get image => isSpanish ? 'Imagen' : 'Images';
  static String get compressed => isSpanish ? 'Comprimidos' : 'Compressed';
  static String get folders => isSpanish ? 'Carpetas' : 'Folders';

  // Menu contextual
  static String get refresh => isSpanish ? 'Actualizar' : 'Refresh';
  static String get open => isSpanish ? 'Abrir' : 'Open';
  static String get revealLocation => isSpanish ? 'Mostrar la ubicación' : 'Show in Explorer';
  static String get copyPath => isSpanish ? 'Copiar la ruta' : 'Copy path';
  static String get copyPaths => isSpanish ? 'Copiar las rutas' : 'Copy paths';
  static String get copyName => isSpanish ? 'Copiar el nombre' : 'Copy name';
  static String get copyNames => isSpanish ? 'Copiar los nombres' : 'Copy names';
  static String get compressZip => isSpanish ? 'Comprimir a archivo ZIP' : 'Compress to ZIP';
  static String get decompressHere => isSpanish ? 'Descomprimir aquí' : 'Extract here';
  static String get newFolder => isSpanish ? 'Nueva carpeta' : 'New folder';
  static String get newFile => isSpanish ? 'Nuevo archivo' : 'New file';

  // Fechas y barra de estado
  static String get today => isSpanish ? 'Hoy' : 'Today';
  static String get yesterday => isSpanish ? 'Ayer' : 'Yesterday';
  static String get items => isSpanish ? 'objetos' : 'items';
  static String get selected => isSpanish ? 'seleccionado' : 'selected';
  static String get selectedPlural => isSpanish ? 'seleccionados' : 'selected';
  static String get indexReady => isSpanish ? 'Índice listo' : 'Index ready';
  static String get indexing => isSpanish ? 'Construyendo índice...' : 'Indexing...';
  static String get serviceStopped =>
      isSpanish ? 'Sin índice: el servicio no está en marcha' : 'No index: background service not running';
  static String get searchPlaceholder => isSpanish ? 'Buscar en' : 'Search in';
  static String get searchEverywhere =>
      isSpanish ? 'Buscar en todo el equipo' : 'Search everywhere';
}
