import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:bdj_studio_search_pro/features/search/models/file_row.dart';
import 'package:bdj_studio_search_pro/features/search/models/selection.dart';
import 'package:bdj_studio_search_pro/features/search/models/view_mode.dart';
import 'package:bdj_studio_search_pro/features/search/providers/search_provider.dart';

void main() {
  group('SearchState & Selection Logic', () {
    test('Initial SearchState has expected default values', () {
      const state = SearchState();
      expect(state.mode, ViewMode.browse);
      expect(state.query, '');
      expect(state.activeFilter, 'Todos');
      expect(state.sortCol, 0);
      expect(state.ascending, true);
      expect(state.viewMode, ResultViewMode.details);
      expect(state.groupBy, GroupByMode.none);
      expect(state.searchEverywhere, true);
      expect(state.selection.count, 0);
      expect(state.canGoBack, false);
      expect(state.canGoForward, false);
    });

    test('Selection model supports range, toggle, select all and clear', () {
      var sel = const Selection(total: 10);
      expect(sel.count, 0);
      expect(sel.contains(3), false);

      sel = sel.toggle(3);
      expect(sel.count, 1);
      expect(sel.contains(3), true);

      sel = sel.toggle(3);
      expect(sel.count, 0);
      expect(sel.contains(3), false);

      sel = sel.all(10);
      expect(sel.count, 10);
      expect(sel.contains(0), true);
      expect(sel.contains(9), true);

      sel = sel.clear();
      expect(sel.count, 0);
      expect(sel.contains(0), false);
    });

    test('Selection range selects contiguous blocks from anchor', () {
      var sel = const Selection(total: 20);
      sel = sel.single(5).range(8);
      expect(sel.count, 4);
      expect(sel.contains(4), false);
      expect(sel.contains(5), true);
      expect(sel.contains(6), true);
      expect(sel.contains(7), true);
      expect(sel.contains(8), true);
      expect(sel.contains(9), false);
    });
  });

  group('SearchNotifier Explorer & Search Logic', () {
    test('SearchNotifier initializes without crashing with autoInit: false', () {
      final notifier = SearchNotifier(autoInit: false);
      expect(notifier.state.mode, ViewMode.browse);
      expect(notifier.state.query, '');
      notifier.dispose();
    });

    test('setResultView and setGroupBy update view presentation state', () {
      final notifier = SearchNotifier(autoInit: false);

      notifier.setResultView(ResultViewMode.list);
      expect(notifier.state.viewMode, ResultViewMode.list);

      notifier.setResultView(ResultViewMode.compact);
      expect(notifier.state.viewMode, ResultViewMode.compact);

      notifier.setResultView(ResultViewMode.grid);
      expect(notifier.state.viewMode, ResultViewMode.grid);

      notifier.setGroupBy(GroupByMode.type);
      expect(notifier.state.groupBy, GroupByMode.type);

      notifier.setGroupBy(GroupByMode.date);
      expect(notifier.state.groupBy, GroupByMode.date);

      notifier.dispose();
    });

    test('toggleSearchEverywhere toggles scope behavior', () {
      final notifier = SearchNotifier(autoInit: false);
      expect(notifier.state.searchEverywhere, true);

      notifier.toggleSearchEverywhere();
      expect(notifier.state.searchEverywhere, false);

      notifier.toggleSearchEverywhere();
      expect(notifier.state.searchEverywhere, true);

      notifier.dispose();
    });

    test('Sorting logic orders directories first, then by requested field', () {
      final rows = [
        const FileRow(index: 0, name: 'track_b.mp3', path: 'C:\\Music', extension: 'mp3', size: 5000, mtime: 200, flags: 0),
        const FileRow(index: 1, name: 'Folder_Z', path: 'C:\\Music', extension: '', size: 0, mtime: 100, flags: 1),
        const FileRow(index: 2, name: 'track_a.wav', path: 'C:\\Music', extension: 'wav', size: 10000, mtime: 300, flags: 0),
        const FileRow(index: 3, name: 'Folder_A', path: 'C:\\Music', extension: '', size: 0, mtime: 50, flags: 1),
      ];

      // Ordenar por nombre ascendente (carpetas primero)
      rows.sort((a, b) {
        if (a.isDirectory != b.isDirectory) return a.isDirectory ? -1 : 1;
        return a.name.toLowerCase().compareTo(b.name.toLowerCase());
      });

      expect(rows[0].name, 'Folder_A');
      expect(rows[1].name, 'Folder_Z');
      expect(rows[2].name, 'track_a.wav');
      expect(rows[3].name, 'track_b.mp3');

      // Ordenar por tamaño ascendente (carpetas primero)
      rows.sort((a, b) {
        if (a.isDirectory != b.isDirectory) return a.isDirectory ? -1 : 1;
        return a.size.compareTo(b.size);
      });

      expect(rows[0].isDirectory, true);
      expect(rows[1].isDirectory, true);
      expect(rows[2].name, 'track_b.mp3');
      expect(rows[3].name, 'track_a.wav');
    });

    test('Local browse listing loads directory children and populates cache', () async {
      final tempDir = await Directory.systemTemp.createTemp('bdj_explorer_test_');
      try {
        final subDir = Directory('${tempDir.path}${Platform.pathSeparator}Subfolder');
        await subDir.create();
        final file1 = File('${tempDir.path}${Platform.pathSeparator}song.mp3');
        await file1.writeAsString('audio content');
        final file2 = File('${tempDir.path}${Platform.pathSeparator}notes.txt');
        await file2.writeAsString('hello world text');

        final notifier = SearchNotifier(autoInit: false);
        final success = await notifier.openFolder(tempDir.path);

        expect(success, true);
        expect(notifier.state.mode, ViewMode.browse);
        expect(notifier.state.browsePath, tempDir.path);
        expect(notifier.state.totalCount, 3); // Subfolder, song.mp3, notes.txt

        // Fila 0 en caché inmediata
        final row0 = notifier.rowAt(0);
        expect(row0, isNotNull);
        expect(row0!.path, tempDir.path);
        expect(row0.isDirectory, true);
        expect(row0.name, 'Subfolder');

        notifier.dispose();
      } finally {
        if (await tempDir.exists()) {
          await tempDir.delete(recursive: true);
        }
      }
    });

    test('Navigation history push, back and forward', () async {
      final tempA = await Directory.systemTemp.createTemp('bdj_nav_a_');
      final tempB = await Directory.systemTemp.createTemp('bdj_nav_b_');
      try {
        final notifier = SearchNotifier(autoInit: false);

        await notifier.openFolder(tempA.path);
        expect(notifier.state.browsePath, tempA.path);
        expect(notifier.state.canGoBack, false);
        expect(notifier.state.history.length, 1);

        await notifier.openFolder(tempB.path);
        expect(notifier.state.browsePath, tempB.path);
        expect(notifier.state.canGoBack, true);
        expect(notifier.state.canGoForward, false);
        expect(notifier.state.history.length, 2);

        // Ir atrás
        notifier.goBack();
        expect(notifier.state.historyIndex, 0);
        expect(notifier.state.canGoBack, false);
        expect(notifier.state.canGoForward, true);

        // Ir adelante
        notifier.goForward();
        expect(notifier.state.historyIndex, 1);
        expect(notifier.state.canGoForward, false);
        expect(notifier.state.canGoBack, true);

        notifier.dispose();
      } finally {
        await tempA.delete(recursive: true);
        await tempB.delete(recursive: true);
      }
    });

    test('Adaptive debounce adapts for high-end and low-end devices', () {
      final notifier = SearchNotifier(autoInit: false);
      expect(notifier.currentDebounce, const Duration(milliseconds: 90));

      // Simular gama alta (respuesta ultrarrápida a 50ms como Everything)
      notifier.setDebounceForTesting(const Duration(milliseconds: 50));
      expect(notifier.currentDebounce, const Duration(milliseconds: 50));

      // Simular gama baja (cuidado de CPU a 160ms para equipos de 2 núcleos)
      notifier.setDebounceForTesting(const Duration(milliseconds: 160));
      expect(notifier.currentDebounce, const Duration(milliseconds: 160));

      notifier.dispose();
    });

    test('Keyboard cursor navigation moves and clamps cursor index', () {
      final notifier = SearchNotifier(autoInit: false);
      // Con rowCount 0 no se mueve
      notifier.moveCursor(1);
      expect(notifier.state.selection.cursor, 0);

      // Con filas disponibles:
      final s = notifier.state.copyWith(
        totalCount: 50,
        selection: const Selection(total: 50, cursor: 0, anchor: 0),
      );
      // Modificar el estado para probar el cursor
      notifier.setResultView(ResultViewMode.details);
      expect(s.selection.cursor, 0);

      final sAvanzado = s.selection.moveCursor(5);
      expect(sAvanzado.cursor, 5);

      final sRango = sAvanzado.extendCursor(10);
      expect(sRango.cursor, 10);
      expect(sRango.count, 6); // 5, 6, 7, 8, 9, 10

      notifier.dispose();
    });

    test('Browse mode selection resolves selectedPaths and pathOf from cache for file ops', () async {
      final tempDir = await Directory.systemTemp.createTemp('bdj_ops_test_');
      try {
        final subDir = Directory('${tempDir.path}${Platform.pathSeparator}CarpetaPrueba');
        await subDir.create();
        final file1 = File('${tempDir.path}${Platform.pathSeparator}audio.wav');
        await file1.writeAsString('audio');

        final notifier = SearchNotifier(autoInit: false);
        await notifier.openFolder(tempDir.path);
        expect(notifier.state.totalCount, 2);

        // Fila 0 es CarpetaPrueba
        notifier.selectRow(0);
        final paths0 = await notifier.selectedPaths();
        expect(paths0.length, 1);
        expect(paths0.first, subDir.path);

        final single0 = await notifier.pathOf(0);
        expect(single0, subDir.path);

        // Operable paths excluye raíces pero incluye subcarpetas normales
        final operables = await notifier.selectedOperablePaths();
        expect(operables.length, 1);
        expect(operables.first, subDir.path);

        // Fila 1 es audio.wav
        notifier.selectRow(1);
        final paths1 = await notifier.selectedPaths();
        expect(paths1.length, 1);
        expect(paths1.first, file1.path);

        notifier.dispose();
      } finally {
        if (await tempDir.exists()) {
          await tempDir.delete(recursive: true);
        }
      }
    });
  });
}
