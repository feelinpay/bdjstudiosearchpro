import 'dart:io';

import 'package:dartz/dartz.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';

import '../errors/failures.dart';
import 'security_port.dart';

/// Opciones de macOS que evitan el llavero de protección de datos, que en
/// aplicaciones de escritorio firmadas fuera de la Mac App Store provoca
/// errSecMissingEntitlement.
class SafeMacOsOptions extends MacOsOptions {
  const SafeMacOsOptions({
    super.accessibility = KeychainAccessibility.first_unlock,
    super.synchronizable = false,
    super.groupId,
    super.usesDataProtectionKeychain = false,
  });

  @override
  Map<String, String> toMap() {
    final map = <String, String>{
      ...super.toMap(),
      'usesDataProtectionKeychain': '$usesDataProtectionKeychain',
      'useDataProtectionKeyChain': '$usesDataProtectionKeychain',
    };
    if (!usesDataProtectionKeychain) {
      map.remove('accessibility');
      map.remove('synchronizable');
      map.remove('groupId');
    }
    return map;
  }
}

/// Almacenamiento seguro nativo con política *fail-closed*: si el llavero del
/// sistema no responde, la activación se bloquea en vez de degradarse a un
/// almacenamiento en claro.
class SecureStorageImpl implements SecurityPort {
  final FlutterSecureStorage _storage;

  static const iOsOptions =
      IOSOptions(accessibility: KeychainAccessibility.first_unlock);
  static const macOsOptions = SafeMacOsOptions(
    accessibility: KeychainAccessibility.first_unlock,
    synchronizable: false,
    groupId: null,
    usesDataProtectionKeychain: false,
  );
  static const windowsOptions = WindowsOptions();
  static const linuxOptions = LinuxOptions();

  SecureStorageImpl({FlutterSecureStorage? storage})
      : _storage = storage ??
            const FlutterSecureStorage(
              iOptions: iOsOptions,
              mOptions: macOsOptions,
              wOptions: windowsOptions,
              lOptions: linuxOptions,
            );

  SecurityFailure _handleError(dynamic e, [String operation = 'unknown']) {
    final msg = e.toString().toLowerCase();
    if (Platform.isMacOS ||
        msg.contains('keychain') ||
        msg.contains('osstatus') ||
        msg.contains('errsec')) {
      // Se redacta cualquier cadena larga del error nativo para no filtrar
      // material de licencia en logs de diagnóstico.
      final sanitized = e
          .toString()
          .replaceAll(RegExp(r'\b([A-Za-z0-9\-_]{24,})\b'), '[SECRETO_REDACTADO]');
      return SecurityFailure(
        'No se pudo usar el llavero nativo de macOS al ejecutar [$operation]. '
        'Detalle: $sanitized',
      );
    }
    if (Platform.isWindows || msg.contains('credential')) {
      return SecurityFailure(
        'No se pudo usar el Administrador de credenciales de Windows al '
        'ejecutar [$operation]. Detalle: $e',
      );
    }
    return SecurityFailure(
      'No se pudo acceder al almacén seguro del sistema en [$operation]: $e',
    );
  }

  @override
  Future<Result<void>> storeSecure(String key, String value) async {
    try {
      await _storage.write(
        key: key,
        value: value,
        iOptions: iOsOptions,
        mOptions: macOsOptions,
        wOptions: windowsOptions,
        lOptions: linuxOptions,
      );
      return const Right(null);
    } catch (e) {
      return Left(_handleError(e, 'write'));
    }
  }

  @override
  Future<Result<String?>> readSecure(String key) async {
    try {
      final value = await _storage.read(
        key: key,
        iOptions: iOsOptions,
        mOptions: macOsOptions,
        wOptions: windowsOptions,
        lOptions: linuxOptions,
      );
      return Right(value);
    } catch (e) {
      return Left(_handleError(e, 'read'));
    }
  }

  @override
  Future<Result<void>> deleteSecure(String key) async {
    try {
      await _storage.delete(
        key: key,
        iOptions: iOsOptions,
        mOptions: macOsOptions,
        wOptions: windowsOptions,
        lOptions: linuxOptions,
      );
      return const Right(null);
    } catch (e) {
      return Left(_handleError(e, 'delete'));
    }
  }

  @override
  Future<Result<bool>> containsSecure(String key) async {
    try {
      final contains = await _storage.containsKey(
        key: key,
        iOptions: iOsOptions,
        mOptions: macOsOptions,
        wOptions: windowsOptions,
        lOptions: linuxOptions,
      );
      return Right(contains);
    } catch (e) {
      return Left(_handleError(e, 'contains'));
    }
  }

  /// Escribe, relee y borra una clave desechable. Una clave con marca de tiempo
  /// evita el errSecDuplicateItem (-25299) que deja el Keychain tras cambios de
  /// firma del binario.
  @override
  Future<Result<bool>> performSelfTest() async {
    final stamp = DateTime.now().millisecondsSinceEpoch;
    final testKey = 'bdj_selftest_$stamp';
    final testValue = 'selftest_$stamp';
    try {
      await _storage.write(
        key: testKey,
        value: testValue,
        iOptions: iOsOptions,
        mOptions: macOsOptions,
        wOptions: windowsOptions,
        lOptions: linuxOptions,
      );
      final readBack = await _storage.read(
        key: testKey,
        iOptions: iOsOptions,
        mOptions: macOsOptions,
        wOptions: windowsOptions,
        lOptions: linuxOptions,
      );
      await _storage.delete(
        key: testKey,
        iOptions: iOsOptions,
        mOptions: macOsOptions,
        wOptions: windowsOptions,
        lOptions: linuxOptions,
      );
      if (readBack != testValue) {
        return const Left(SecurityFailure(
          'El almacén seguro devolvió un valor distinto al escrito. '
          'Activación bloqueada por seguridad.',
        ));
      }
      final checkDeleted = await _storage.read(
        key: testKey,
        iOptions: iOsOptions,
        mOptions: macOsOptions,
        wOptions: windowsOptions,
        lOptions: linuxOptions,
      );
      if (checkDeleted != null) {
        return const Left(SecurityFailure(
          'El almacén seguro no confirmó el borrado de la clave de prueba. '
          'Activación bloqueada por seguridad.',
        ));
      }
      return const Right(true);
    } catch (e) {
      return Left(_handleError(e, 'self_test'));
    }
  }
}
