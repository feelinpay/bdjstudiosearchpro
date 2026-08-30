import 'package:bdj_license_core/bdj_license_core.dart';
import 'package:dartz/dartz.dart';
import 'package:package_info_plus/package_info_plus.dart';

import '../errors/failures.dart';
import '../security/device_fingerprint.dart';
import '../security/security_port.dart';
import 'licensing_port.dart';

/// Claves del almacén seguro. El espacio de nombres `bdj.search_pro.*` aísla
/// esta aplicación del resto de la suite: ninguna app puede leer ni pisar la
/// licencia de otra.
class LicenseStorageKeys {
  static const String licenseKey = 'bdj.search_pro.license_key';
  static const String licenseStatus = 'bdj.search_pro.license_status';
  static const String deviceId = 'bdj.search_pro.device_id';
  static const String hardwareFingerprint =
      'bdj.search_pro.hardware_fingerprint';
  static const String lastLicenseCheckUtc =
      'bdj.search_pro.last_license_check_utc';
  static const String lastSyncAt = 'bdj.search_pro.last_sync_at';
  static const String installId = 'bdj.search_pro.install_id';

  // Search Pro es un producto nuevo: no existen claves legadas que migrar.
}

/// Verificación de licencia SPP3 contra la clave raíz del ecosistema BDJ Studio.
///
/// Toda la criptografía vive en `bdj_license_core` (Ed25519 sobre el certificado
/// de administrador y sobre el token). Esta clase solo orquesta: obtiene la
/// huella de hardware, resuelve la versión exacta y persiste el resultado en el
/// llavero nativo.
class LicenseManager implements LicensingPort {
  final SecurityPort secureStorage;
  final DeviceFingerprint fingerprint;
  final String productCode;
  final String defaultAppVersion;

  LicenseStatus _currentStatus = LicenseStatus.none;
  String? _cachedFingerprint;
  String? _cachedAppVersion;

  LicenseManager({
    required this.secureStorage,
    required this.fingerprint,
    this.productCode = 'bdj_studio_search_pro',
    this.defaultAppVersion = '1.0.0',
  });

  @override
  LicenseStatus get currentStatus => _currentStatus;

  @override
  bool get isLicensed => _currentStatus == LicenseStatus.active;

  Future<String> _getAppVersion() async {
    if (_cachedAppVersion != null) return _cachedAppVersion!;
    try {
      final packageInfo = await PackageInfo.fromPlatform();
      if (packageInfo.version.isNotEmpty) {
        _cachedAppVersion = packageInfo.version;
        return _cachedAppVersion!;
      }
    } catch (_) {}
    _cachedAppVersion = defaultAppVersion;
    return _cachedAppVersion!;
  }

  /// Huella visible del equipo (`XXXX-XXXX-XXXX-XXXX`). Es el código que el
  /// cliente copia y te envía para que emitas su licencia, y es idéntico al que
  /// muestran las demás apps de la suite en la misma máquina.
  Future<String> getHardwareFingerprint() async {
    if (_cachedFingerprint != null) return _cachedFingerprint!;
    _cachedFingerprint = await fingerprint.generate();
    await secureStorage.storeSecure(
      LicenseStorageKeys.hardwareFingerprint,
      _cachedFingerprint!,
    );
    return _cachedFingerprint!;
  }

  /// Resuelve el HWID convirtiendo cualquier fallo del hardware en un `Failure`.
  ///
  /// En un equipo con WMI bloqueado o con `device_info` sin responder, esto
  /// antes salía como excepción sin capturar y tumbaba la activación con un
  /// error crudo. Ahora llega a la interfaz como un mensaje que el usuario
  /// puede entender y reintentar.
  Future<Result<String>> _resolveHwid() async {
    try {
      return Right(await getHardwareFingerprint());
    } catch (e) {
      return Left(DeviceFailure(
        'No se pudo leer el identificador de hardware de este equipo: $e',
      ));
    }
  }

  @override
  Future<Result<LicenseInfo>> activateLicense(String licenseKey) async {
    final cleanKey = licenseKey.replaceAll(RegExp(r'\s+'), '');

    // La comprobación de formato va primero por ser la más barata: una clave
    // que no es SPP3 se rechaza sin tocar el llavero ni consultar el hardware.
    if (!cleanKey.startsWith('SPP3.')) {
      _currentStatus = LicenseStatus.invalid;
      return const Left(LicenseFailure(
        'El código ingresado no corresponde al formato oficial de licencia '
        'segura SPP3.',
      ));
    }

    // El almacén seguro debe funcionar ANTES de aceptar una activación: si no
    // podemos persistir el resultado, activar solo produciría una licencia que
    // se pierde al cerrar la aplicación.
    final selfTest = await secureStorage.performSelfTest();
    if (selfTest.isLeft()) {
      _currentStatus = LicenseStatus.invalid;
      return Left(
        selfTest.fold(
          (l) => l,
          (r) => const SecurityFailure(
            'Error verificando el almacén seguro nativo del sistema.',
          ),
        ),
      );
    }

    final hwidResult = await _resolveHwid();
    if (hwidResult.isLeft()) {
      _currentStatus = LicenseStatus.invalid;
      return Left(hwidResult.fold(
        (l) => l,
        (r) => const DeviceFailure('Identificador de hardware no disponible.'),
      ));
    }

    final hwid = hwidResult.getOrElse(() => '');
    final appVersion = await _getAppVersion();
    return _verifySpp3(cleanKey, hwid, appVersion, persist: true);
  }

  @override
  Future<Result<LicenseInfo>> validateLicense() async {
    final storedKey =
        (await secureStorage.readSecure(LicenseStorageKeys.licenseKey))
            .getOrElse(() => null);
    final storedStatus =
        (await secureStorage.readSecure(LicenseStorageKeys.licenseStatus))
            .getOrElse(() => null);

    if (storedKey == null || storedKey.isEmpty) {
      _currentStatus = LicenseStatus.none;
      return const Left(
        LicenseFailure('No hay una licencia activa en este dispositivo.'),
      );
    }

    if (storedStatus != LicenseStatus.active.name ||
        !storedKey.startsWith('SPP3.')) {
      _currentStatus = LicenseStatus.invalid;
      return const Left(LicenseFailure(
        'Estado de licencia inválido. Vuelve a activar tu clave SPP3.',
      ));
    }

    // Revalidación criptográfica completa en cada arranque: una licencia
    // guardada no se cree por estar guardada.
    final hwidResult = await _resolveHwid();
    if (hwidResult.isLeft()) {
      _currentStatus = LicenseStatus.invalid;
      return Left(hwidResult.fold(
        (l) => l,
        (r) => const DeviceFailure('Identificador de hardware no disponible.'),
      ));
    }

    final hwid = hwidResult.getOrElse(() => '');
    final appVersion = await _getAppVersion();
    final verification =
        await _verifySpp3(storedKey, hwid, appVersion, persist: true);
    if (verification.isLeft()) {
      _currentStatus = LicenseStatus.invalid;
    }
    return verification;
  }

  Future<Result<LicenseInfo>> _verifySpp3(
    String licenseKey,
    String hwid,
    String appVersion, {
    required bool persist,
  }) async {
    try {
      final hwidHash = KeyHierarchy.hashHwid(hwid);
      final result = await Spp3Token.verify(
        token: licenseKey,
        rootPublicKeyBase64: KeyHierarchy.ecosystemRootPublicKey,
        expectedProductCode: productCode,
        expectedVersion: appVersion,
        currentHwidHash: hwidHash,
      );

      if (!result.isValid) {
        _currentStatus = LicenseStatus.invalid;
        return Left(LicenseFailure(
          result.errorMessage ??
              'Validación criptográfica SPP3 rechazada por el sistema.',
        ));
      }

      final payload = result.payload!;
      final now = DateTime.now().toUtc();

      // Protección anti-retroceso de reloj: adelantar la fecha para caducar y
      // luego retroceder no revive una licencia vencida.
      final lastCheck = DateTime.tryParse(
        (await secureStorage
                    .readSecure(LicenseStorageKeys.lastLicenseCheckUtc))
                .getOrElse(() => null) ??
            '',
      )?.toUtc();
      if (payload.expiresAtUtc != null &&
          lastCheck != null &&
          now.isBefore(lastCheck.subtract(const Duration(minutes: 5)))) {
        _currentStatus = LicenseStatus.invalid;
        return const Left(LicenseFailure(
          'La fecha y hora del sistema retrocedió de forma anormal. '
          'Ajusta tu reloj a la hora real.',
        ));
      }

      final remainingDays = payload.expiresAtUtc == null
          ? 3650
          : (payload.expiresAtUtc!.difference(now).inHours / 24).ceil();

      if (persist) {
        await _saveActivationData(
          licenseKey: licenseKey,
          status: LicenseStatus.active.name,
          deviceId: hwid,
        );
        await secureStorage.storeSecure(
          LicenseStorageKeys.lastLicenseCheckUtc,
          now.toIso8601String(),
        );
      }

      _currentStatus = LicenseStatus.active;
      return Right(LicenseInfo(
        licenseKey: licenseKey,
        status: LicenseStatus.active,
        activatedAt: payload.issuedAtUtc,
        expiresAt: payload.expiresAtUtc,
        deviceId: hwid,
        remainingOfflineDays: remainingDays > 0 ? remainingDays : 0,
      ));
    } catch (e) {
      _currentStatus = LicenseStatus.invalid;
      return Left(LicenseFailure(
        'Error interno durante la verificación criptográfica SPP3: $e',
      ));
    }
  }

  @override
  Future<Result<void>> deactivateLicense() async {
    await secureStorage.deleteSecure(LicenseStorageKeys.licenseKey);
    await secureStorage.deleteSecure(LicenseStorageKeys.licenseStatus);
    await secureStorage.deleteSecure(LicenseStorageKeys.lastSyncAt);
    await secureStorage.deleteSecure(LicenseStorageKeys.lastLicenseCheckUtc);
    _currentStatus = LicenseStatus.none;
    return const Right(null);
  }

  @override
  Future<Result<LicenseInfo>> syncLicense() => validateLicense();

  Future<void> _saveActivationData({
    required String licenseKey,
    required String status,
    required String deviceId,
  }) async {
    await secureStorage.storeSecure(LicenseStorageKeys.licenseKey, licenseKey);
    await secureStorage.storeSecure(LicenseStorageKeys.licenseStatus, status);
    await secureStorage.storeSecure(LicenseStorageKeys.deviceId, deviceId);
    await secureStorage.storeSecure(
      LicenseStorageKeys.lastSyncAt,
      DateTime.now().toIso8601String(),
    );
  }
}
