import 'package:bdj_studio_search_pro/core/errors/failures.dart';
import 'package:bdj_studio_search_pro/core/security/security_port.dart';
import 'package:bdj_studio_search_pro/features/licensing/presentation/providers/license_providers.dart';
import 'package:bdj_studio_search_pro/features/licensing/presentation/screens/activation_screen.dart';
import 'package:bdj_studio_search_pro/main.dart';
import 'package:dartz/dartz.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

/// Almacén seguro en memoria. Deja correr el `LicenseManager` real —queremos
/// probar el portón de verdad, no una imitación— sin tocar el llavero nativo.
class _InMemorySecureStorage implements SecurityPort {
  final Map<String, String> _values = {};

  @override
  Future<Result<void>> storeSecure(String key, String value) async {
    _values[key] = value;
    return const Right(null);
  }

  @override
  Future<Result<String?>> readSecure(String key) async => Right(_values[key]);

  @override
  Future<Result<void>> deleteSecure(String key) async {
    _values.remove(key);
    return const Right(null);
  }

  @override
  Future<Result<bool>> containsSecure(String key) async =>
      Right(_values.containsKey(key));

  @override
  Future<Result<bool>> performSelfTest() async => const Right(true);
}

Widget _appWithStorage([_InMemorySecureStorage? storage]) {
  return ProviderScope(
    overrides: [
      secureStorageProvider
          .overrideWithValue(storage ?? _InMemorySecureStorage()),
    ],
    child: const SearchProApp(),
  );
}

/// Avanza varios fotogramas sin usar `pumpAndSettle`.
///
/// `pumpAndSettle` espera a que no quede ninguna animación pendiente y aquí eso
/// no ocurre nunca: el campo de la licencia tiene el foco y su cursor parpadea
/// en bucle, igual que el indicador de progreso mientras se calcula el ID.
Future<void> _pumpFrames(WidgetTester tester, {int frames = 10}) async {
  for (var i = 0; i < frames; i++) {
    await tester.pump(const Duration(milliseconds: 50));
  }
}

void main() {
  testWidgets(
    'sin licencia guardada, la app se detiene en la pantalla de activación',
    (tester) async {
      await tester.pumpWidget(_appWithStorage());
      await _pumpFrames(tester);

      expect(find.byType(ActivationScreen), findsOneWidget);
      expect(find.byType(SearchHomeScreen), findsNothing);
      expect(find.text('Activar aplicación'), findsOneWidget);
      expect(find.text('ID DEL DISPOSITIVO'), findsOneWidget);
    },
  );

  testWidgets(
    'una clave que no empieza por SPP3 es rechazada y no abre la aplicación',
    (tester) async {
      await tester.pumpWidget(_appWithStorage());
      await _pumpFrames(tester);

      // La forma exacta del esquema propietario que se eliminó del motor Rust.
      // Si esta prueba deja de fallar la activación, hemos vuelto atrás.
      await tester.enterText(find.byType(TextField), 'BDJ6-0000-0000-0000');
      await tester.tap(find.text('Activar aplicación'));
      await _pumpFrames(tester);

      expect(find.byType(SearchHomeScreen), findsNothing);
      expect(find.byType(ActivationScreen), findsOneWidget);
      expect(
        find.textContaining('no corresponde al formato oficial'),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    'una licencia falsificada en el almacén no abre la aplicación',
    (tester) async {
      // Alguien que edite el llavero a mano y ponga una clave del esquema
      // antiguo marcada como activa: la validación de arranque debe rechazarla
      // antes de mirar siquiera el hardware.
      final storage = _InMemorySecureStorage();
      await storage.storeSecure(
        'bdj.search_pro.license_key',
        'BDJ6-0000-0000-0000',
      );
      await storage.storeSecure('bdj.search_pro.license_status', 'active');

      await tester.pumpWidget(_appWithStorage(storage));
      await _pumpFrames(tester);

      expect(find.byType(SearchHomeScreen), findsNothing);
      expect(find.byType(ActivationScreen), findsOneWidget);
    },
  );
}
