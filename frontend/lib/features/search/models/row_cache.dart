import 'file_row.dart';

/// Ventana de filas realmente cargadas, acotada.
///
/// La versión anterior acumulaba: cada vez que el usuario bajaba, se añadían
/// doscientas filas más a una lista que no se recortaba nunca. Bajar hasta el
/// resultado un millón retenía un millón de `FileRow`, cada uno con sus tres
/// cadenas. Eso no es virtualizar, es acumular.
///
/// Aquí las filas se guardan por páginas y se expulsa la página menos usada en
/// cuanto se pasa del tope. La memoria queda acotada por `maxPages` sin importar
/// cuántos resultados haya: dos mil filas, siempre.
class RowCache {
  RowCache({this.pageSize = 200, int maxPages = 10}) : _maxPages = maxPages;

  final int pageSize;

  int _maxPages;

  /// Cuántas páginas caben en memoria.
  int get maxPages => _maxPages;

  /// Ajusta el tope al perfil del equipo y expulsa lo que ya no cabe.
  ///
  /// En un portátil de gama baja, dos mil filas retenidas —cada una con sus
  /// tres cadenas— son memoria que hace falta en otro sitio.
  void resize({required int maxPages}) {
    _maxPages = maxPages.clamp(2, 64);
    _desalojarSobrantes();
  }

  final Map<int, List<FileRow>> _pages = {};
  /// Páginas de la menos usada a la más reciente.
  final List<int> _lru = [];
  final Set<int> _inFlight = {};

  int pageOf(int index) => index ~/ pageSize;

  /// La fila, si su página está cargada. `null` significa «pídemela».
  FileRow? rowAt(int index) {
    if (index < 0) return null;
    final page = pageOf(index);
    final rows = _pages[page];
    if (rows == null) return null;
    _touch(page);
    final offset = index - page * pageSize;
    if (offset < 0 || offset >= rows.length) return null;
    return rows[offset];
  }

  bool hasPage(int page) => _pages.containsKey(page);
  bool isLoading(int page) => _inFlight.contains(page);

  void markLoading(int page) => _inFlight.add(page);

  void put(int page, List<FileRow> rows) {
    _inFlight.remove(page);
    _pages[page] = rows;
    _touch(page);
    _desalojarSobrantes();
  }

  void _desalojarSobrantes() {
    while (_pages.length > _maxPages && _lru.isNotEmpty) {
      final vieja = _lru.removeAt(0);
      _pages.remove(vieja);
    }
  }

  void failed(int page) => _inFlight.remove(page);

  void clear() {
    _pages.clear();
    _lru.clear();
    _inFlight.clear();
  }

  int get loadedRows => _pages.values.fold(0, (a, b) => a + b.length);

  void _touch(int page) {
    _lru.remove(page);
    _lru.add(page);
  }
}
