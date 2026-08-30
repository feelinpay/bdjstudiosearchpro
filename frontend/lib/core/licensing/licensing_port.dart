import '../errors/failures.dart';

enum LicenseStatus { active, expired, invalid, offline, none }

class LicenseInfo {
  final String licenseKey;
  final LicenseStatus status;
  final DateTime? activatedAt;
  final DateTime? expiresAt;
  final String? deviceId;
  final int remainingOfflineDays;

  const LicenseInfo({
    required this.licenseKey,
    required this.status,
    this.activatedAt,
    this.expiresAt,
    this.deviceId,
    this.remainingOfflineDays = 30,
  });
}

abstract class LicensingPort {
  Future<Result<LicenseInfo>> activateLicense(String licenseKey);
  Future<Result<LicenseInfo>> validateLicense();
  Future<Result<void>> deactivateLicense();
  Future<Result<LicenseInfo>> syncLicense();
  LicenseStatus get currentStatus;
  bool get isLicensed;
}
