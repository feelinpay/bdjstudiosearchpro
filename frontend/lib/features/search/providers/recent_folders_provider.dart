import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// Almacena y persiste las últimas 10 carpetas navegadas en el explorador.
class RecentFoldersNotifier extends StateNotifier<List<String>> {
  RecentFoldersNotifier() : super(const []) {
    _load();
  }

  static const String _prefsKey = 'sidebar_recent_folders';
  static const int _maxRecents = 10;

  Future<void> _load() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      final saved = prefs.getStringList(_prefsKey);
      if (saved != null) {
        state = saved;
      }
    } catch (_) {}
  }

  Future<void> _persist() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      await prefs.setStringList(_prefsKey, state);
    } catch (_) {}
  }

  /// Registra una carpeta recién navegada en la cima del historial.
  Future<void> add(String path) async {
    final trimmed = path.trim();
    if (trimmed.isEmpty) return;
    final updated = List<String>.from(state)..remove(trimmed);
    updated.insert(0, trimmed);
    if (updated.length > _maxRecents) {
      updated.removeRange(_maxRecents, updated.length);
    }
    state = updated;
    await _persist();
  }

  /// Limpia el historial de carpetas recientes.
  Future<void> clear() async {
    state = const [];
    await _persist();
  }
}

final recentFoldersProvider =
    StateNotifierProvider<RecentFoldersNotifier, List<String>>(
  (ref) => RecentFoldersNotifier(),
);
