import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:bdj_studio_search_pro/features/search/providers/favorites_provider.dart';
import 'package:bdj_studio_search_pro/features/search/providers/recent_folders_provider.dart';
import 'package:bdj_studio_search_pro/features/search/providers/search_provider.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  group('Pruebas reales de Marcadores y Carpetas Recientes', () {
    setUp(() {
      SharedPreferences.setMockInitialValues({});
    });

    test('Marcadores: añadir, mover arriba/abajo, eliminar y persistir', () async {
      final favNotifier = FavoritesNotifier();
      await Future<void>.delayed(const Duration(milliseconds: 50));

      // 1. Añadir marcadores
      await favNotifier.add('C:\\Musica_DJ');
      await favNotifier.add('C:\\Samples_Ableton');
      await favNotifier.add('C:\\Rekordbox_Sets');

      expect(favNotifier.state.contains('C:\\Musica_DJ'), true);
      expect(favNotifier.state.contains('C:\\Samples_Ableton'), true);
      expect(favNotifier.state.contains('C:\\Rekordbox_Sets'), true);

      // No duplicar si ya existe
      final totalAntes = favNotifier.state.length;
      await favNotifier.add('C:\\Musica_DJ');
      expect(favNotifier.state.length, totalAntes);

      // 2. Mover hacia arriba y hacia abajo
      final idxSamplesAntes = favNotifier.state.indexOf('C:\\Samples_Ableton');
      await favNotifier.moveUp('C:\\Samples_Ableton');
      expect(favNotifier.state.indexOf('C:\\Samples_Ableton'), idxSamplesAntes - 1);

      await favNotifier.moveDown('C:\\Samples_Ableton');
      expect(favNotifier.state.indexOf('C:\\Samples_Ableton'), idxSamplesAntes);

      // 3. Eliminar marcador
      await favNotifier.remove('C:\\Musica_DJ');
      expect(favNotifier.state.contains('C:\\Musica_DJ'), false);

      // 4. Comprobar persistencia en SharedPreferences
      final prefs = await SharedPreferences.getInstance();
      final guardados = prefs.getStringList('sidebar_favorites');
      expect(guardados, isNotNull);
      expect(guardados!.contains('C:\\Samples_Ableton'), true);
      expect(guardados.contains('C:\\Musica_DJ'), false);
    });

    test('Carpetas Recientes: registrar navegación, límite de 10 y limpiar', () async {
      final recents = RecentFoldersNotifier();
      await Future<void>.delayed(const Duration(milliseconds: 50));

      for (var i = 1; i <= 15; i++) {
        await recents.add('C:\\Folder_$i');
      }

      // Máximo 10 carpetas
      expect(recents.state.length, 10);
      // La última añadida debe estar en la cima (índice 0)
      expect(recents.state.first, 'C:\\Folder_15');

      await recents.clear();
      expect(recents.state, isEmpty);
    });
  });

  group('Pruebas reales de Explorador y Operaciones con Archivos Reales', () {
    late Directory tempWorkspace;

    setUp(() async {
      tempWorkspace = await Directory.systemTemp.createTemp('bdj_real_test_');
    });

    tearDown(() async {
      if (await tempWorkspace.exists()) {
        await tempWorkspace.delete(recursive: true);
      }
    });

    test('Flujo completo de explorador: crear carpeta, crear archivo, copiar ruta, renombrar, duplicar y eliminar', () async {
      // 1. Crear carpeta real en disco
      final sesionDir = Directory('${tempWorkspace.path}${Platform.pathSeparator}Sesion_Live_2026');
      await sesionDir.create();
      expect(await sesionDir.exists(), true);

      // 2. Crear archivos reales
      final track1 = File('${sesionDir.path}${Platform.pathSeparator}pista_01.mp3');
      await track1.writeAsString('audio-mp3-stream-data');

      final track2 = File('${sesionDir.path}${Platform.pathSeparator}pista_02.wav');
      await track2.writeAsString('audio-wav-stream-data');

      final notas = File('${sesionDir.path}${Platform.pathSeparator}cues.txt');
      await notas.writeAsString('Cue 1: 01:23, Cue 2: 02:45');

      // 3. Abrir la carpeta con el explorador SearchNotifier
      final searchNotifier = SearchNotifier(autoInit: false);
      final ok = await searchNotifier.openFolder(sesionDir.path);
      expect(ok, true);

      expect(searchNotifier.state.browsePath, sesionDir.path);
      expect(searchNotifier.state.totalCount, 3);

      // 4. Copia de ruta: verificar que las rutas leídas son reales y existen
      final rutas = <String>[];
      for (var i = 0; i < searchNotifier.state.totalCount; i++) {
        final row = searchNotifier.rowAt(i);
        expect(row, isNotNull);
        final fullPath = '${row!.path}${Platform.pathSeparator}${row.name}';
        rutas.add(fullPath);
        expect(File(fullPath).existsSync(), true);
      }
      expect(rutas.length, 3);

      // 5. Duplicar archivo en disco (comportamiento Explorer con sufijo)
      final track1Duplicado = File('${sesionDir.path}${Platform.pathSeparator}pista_01 (2).mp3');
      await track1.copy(track1Duplicado.path);
      expect(await track1Duplicado.exists(), true);

      // Refrescar explorador
      await searchNotifier.openFolder(sesionDir.path);
      expect(searchNotifier.state.totalCount, 4);

      // 6. Renombrar archivo en disco
      final track1Renombrado = File('${sesionDir.path}${Platform.pathSeparator}pista_01_remix.mp3');
      await track1Duplicado.rename(track1Renombrado.path);
      expect(await track1Duplicado.exists(), false);
      expect(await track1Renombrado.exists(), true);

      // Refrescar explorador
      await searchNotifier.openFolder(sesionDir.path);
      expect(searchNotifier.state.totalCount, 4);

      // 7. Eliminar archivo permanentemente
      await track1Renombrado.delete();
      expect(await track1Renombrado.exists(), false);

      await searchNotifier.openFolder(sesionDir.path);
      expect(searchNotifier.state.totalCount, 3);

      searchNotifier.dispose();
    });

    test('Filtros avanzados y modo consulta sobre explorador', () {
      final notifier = SearchNotifier(autoInit: false);

      // Filtro de Audio
      notifier.setFilter('Audio');
      expect(notifier.state.activeFilter, 'Audio');

      // Filtro de Proyectos DJ
      notifier.setFilter('Proyectos DJ');
      expect(notifier.state.activeFilter, 'Proyectos DJ');

      // Filtro de Vídeo
      notifier.setFilter('Vídeo');
      expect(notifier.state.activeFilter, 'Vídeo');

      // Filtro de Documentos
      notifier.setFilter('Documentos');
      expect(notifier.state.activeFilter, 'Documentos');

      // Filtro de Comprimidos
      notifier.setFilter('Comprimidos');
      expect(notifier.state.activeFilter, 'Comprimidos');

      // Flags de búsqueda avanzada
      notifier.toggleCaseSensitive();
      expect(notifier.state.isCaseSensitive, true);

      notifier.togglePathMatch();
      expect(notifier.state.isPathMatch, true);

      notifier.toggleRegex();
      expect(notifier.state.isRegex, true);

      notifier.dispose();
    });
  });
}
