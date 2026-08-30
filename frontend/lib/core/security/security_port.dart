import '../errors/failures.dart';

/// Contrato de almacenamiento seguro nativo (Credential Manager en Windows,
/// Keychain en macOS). Idéntico al de Sample Pad: no modificar sin alinear
/// el resto de la suite.
abstract class SecurityPort {
  Future<Result<void>> storeSecure(String key, String value);
  Future<Result<String?>> readSecure(String key);
  Future<Result<void>> deleteSecure(String key);
  Future<Result<bool>> containsSecure(String key);
  Future<Result<bool>> performSelfTest();
}
