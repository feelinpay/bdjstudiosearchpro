import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// Fuente de verdad única para los marcadores (favoritos de carpetas).
///
/// Antes cada widget guardaba su propia lista en `SharedPreferences` y se
/// quedaba desincronizado: añadir desde el menú no aparecía en el panel
/// lateral ni al revés. Aquí vive la lista, se persiste al cambiarla y tanto la
/// barra de menús como el panel lateral la observan.
class FavoritesNotifier extends StateNotifier<List<String>> {
  FavoritesNotifier() : super(const []) {
    _load();
  }

  static const String _prefsKey = 'sidebar_favorites';

  /// Lista por defecto que se enseña la primera vez, para que el panel no salga
  /// vacío de serie.
  static const List<String> _defaults = ['C:\\Users'];

  Future<void> _load() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      final saved = prefs.getStringList(_prefsKey);
      state = saved != null && saved.isNotEmpty ? saved : List.of(_defaults);
    } catch (_) {
      state = List.of(_defaults);
    }
  }

  Future<void> _persist() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      await prefs.setStringList(_prefsKey, state);
    } catch (_) {
      // Un fallo de persistencia no debe tumbar la interfaz.
    }
  }

  /// Añade una carpeta al final si aún no estaba.
  Future<void> add(String path) async {
    final recortada = path.trim();
    if (recortada.isEmpty) return;
    if (state.contains(recortada)) return;
    state = [...state, recortada];
    await _persist();
  }

  /// Elimina una carpeta de los marcadores.
  Future<void> remove(String path) async {
    final index = state.indexOf(path);
    if (index < 0) return;
    state = [...state]..removeAt(index);
    await _persist();
  }

  /// Sube un marcador una posición en la lista.
  Future<void> moveUp(String path) async {
    final index = state.indexOf(path);
    if (index <= 0) return;
    final lista = [...state];
    final item = lista.removeAt(index);
    lista.insert(index - 1, item);
    state = lista;
    await _persist();
  }

  /// Baja un marcador una posición en la lista.
  Future<void> moveDown(String path) async {
    final index = state.indexOf(path);
    if (index < 0 || index >= state.length - 1) return;
    final lista = [...state];
    final item = lista.removeAt(index);
    lista.insert(index + 1, item);
    state = lista;
    await _persist();
  }
}

final favoritesProvider =
    StateNotifierProvider<FavoritesNotifier, List<String>>(
  (ref) => FavoritesNotifier(),
);
