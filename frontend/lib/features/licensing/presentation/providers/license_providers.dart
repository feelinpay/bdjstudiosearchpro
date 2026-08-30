import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../../core/errors/failures.dart';
import '../../../../core/licensing/license_manager.dart';
import '../../../../core/licensing/licensing_port.dart';
import '../../../../core/security/device_fingerprint.dart';
import '../../../../core/security/secure_storage_impl.dart';
import '../../../../core/security/security_port.dart';

/// Se expone como `SecurityPort`, no como la implementación concreta, para que
/// las pruebas puedan sustituirlo por un almacén en memoria sin tocar el
/// llavero nativo del sistema.
final secureStorageProvider = Provider<SecurityPort>((ref) {
  return SecureStorageImpl();
});

final deviceFingerprintProvider = Provider<DeviceFingerprint>((ref) {
  return DeviceFingerprint.withPersistentStorage(
    ref.read(secureStorageProvider),
  );
});

final licenseManagerProvider = Provider<LicenseManager>((ref) {
  return LicenseManager(
    secureStorage: ref.read(secureStorageProvider),
    fingerprint: ref.read(deviceFingerprintProvider),
  );
});

enum LicenseLoadingState { initial, loading, licensed, unlicensed, error }

class LicenseState {
  final LicenseLoadingState loadingState;
  final LicenseStatus status;
  final String? error;
  final String? licenseKey;
  final DateTime? activatedAt;
  final DateTime? expiresAt;
  final String? deviceId;
  final int remainingOfflineDays;

  const LicenseState({
    this.loadingState = LicenseLoadingState.initial,
    this.status = LicenseStatus.none,
    this.error,
    this.licenseKey,
    this.activatedAt,
    this.expiresAt,
    this.deviceId,
    this.remainingOfflineDays = 30,
  });

  LicenseState copyWith({
    LicenseLoadingState? loadingState,
    LicenseStatus? status,
    String? error,
    String? licenseKey,
    DateTime? activatedAt,
    DateTime? expiresAt,
    String? deviceId,
    int? remainingOfflineDays,
  }) {
    return LicenseState(
      loadingState: loadingState ?? this.loadingState,
      status: status ?? this.status,
      // `error` se pasa siempre para poder limpiarlo al reintentar.
      error: error,
      licenseKey: licenseKey ?? this.licenseKey,
      activatedAt: activatedAt ?? this.activatedAt,
      expiresAt: expiresAt ?? this.expiresAt,
      deviceId: deviceId ?? this.deviceId,
      remainingOfflineDays: remainingOfflineDays ?? this.remainingOfflineDays,
    );
  }
}

class LicenseNotifier extends StateNotifier<LicenseState> {
  final LicenseManager _manager;

  /// Presupuesto máximo para validar la licencia al arrancar.
  ///
  /// La validación es offline y criptográfica, pero lee del llavero nativo y
  /// genera la huella HWID, que en Windows implica una consulta WMI vía
  /// PowerShell. En equipos lentos eso puede tardar; sin límite la aplicación
  /// se quedaría en el indicador de progreso para siempre. Con él, cae a la
  /// pantalla de activación con un mensaje y el usuario puede reintentar.
  static const _checkBudget = Duration(seconds: 12);

  LicenseNotifier(this._manager) : super(const LicenseState()) {
    _checkLicense();
  }

  Future<void> _checkLicense() async {
    state = state.copyWith(loadingState: LicenseLoadingState.loading);

    final Result<LicenseInfo> result;
    try {
      result = await _manager.validateLicense().timeout(_checkBudget);
    } catch (_) {
      state = state.copyWith(
        loadingState: LicenseLoadingState.error,
        status: _manager.currentStatus,
        error: 'La verificación de licencia tardó demasiado. '
            'Revisa el equipo e inténtalo de nuevo.',
      );
      return;
    }

    result.fold(
      (failure) {
        state = state.copyWith(
          loadingState: LicenseLoadingState.unlicensed,
          status: _manager.currentStatus,
          // Sin licencia previa no hay error que mostrar: es un primer arranque.
          error: _manager.currentStatus == LicenseStatus.none
              ? null
              : failure.message,
        );
      },
      (info) {
        state = state.copyWith(
          loadingState: LicenseLoadingState.licensed,
          status: info.status,
          licenseKey: info.licenseKey,
          activatedAt: info.activatedAt,
          expiresAt: info.expiresAt,
          deviceId: info.deviceId,
          remainingOfflineDays: info.remainingOfflineDays,
        );
      },
    );
  }

  Future<void> activate(String licenseKey) async {
    state = state.copyWith(
      loadingState: LicenseLoadingState.loading,
      error: null,
    );

    final result = await _manager.activateLicense(licenseKey);
    result.fold(
      (failure) {
        state = state.copyWith(
          loadingState: LicenseLoadingState.error,
          status: _manager.currentStatus,
          error: failure.message,
        );
      },
      (info) {
        state = state.copyWith(
          loadingState: LicenseLoadingState.licensed,
          status: info.status,
          licenseKey: info.licenseKey,
          activatedAt: info.activatedAt,
          expiresAt: info.expiresAt,
          deviceId: info.deviceId,
          remainingOfflineDays: info.remainingOfflineDays,
        );
      },
    );
  }

  Future<void> sync() async {
    final result = await _manager.syncLicense();
    result.fold(
      (failure) {
        // Una licencia que deja de validar expulsa al usuario a la pantalla de
        // activación: no se sigue usando la aplicación con licencia inválida.
        state = state.copyWith(
          loadingState: LicenseLoadingState.unlicensed,
          status: _manager.currentStatus,
          error: failure.message,
        );
      },
      (info) {
        state = state.copyWith(
          loadingState: LicenseLoadingState.licensed,
          status: info.status,
          remainingOfflineDays: info.remainingOfflineDays,
        );
      },
    );
  }

  Future<void> deactivate() async {
    await _manager.deactivateLicense();
    state = const LicenseState(
      loadingState: LicenseLoadingState.unlicensed,
      status: LicenseStatus.none,
    );
  }
}

final licenseProvider =
    StateNotifierProvider<LicenseNotifier, LicenseState>((ref) {
  return LicenseNotifier(ref.read(licenseManagerProvider));
});
