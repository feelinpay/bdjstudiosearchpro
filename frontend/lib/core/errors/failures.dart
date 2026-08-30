import 'package:dartz/dartz.dart';

/// Jerarquía de fallos compartida con el resto de la suite BDJ Studio.
/// Se mantiene idéntica a la de Sample Pad para que el código de licenciamiento
/// portado compile sin adaptaciones.
sealed class Failure {
  final String message;
  final String? code;
  final StackTrace? stackTrace;

  const Failure(this.message, {this.code, this.stackTrace});

  @override
  String toString() => '$runtimeType(message: $message, code: $code)';
}

class LicenseFailure extends Failure {
  const LicenseFailure(super.message, {super.code, super.stackTrace});
}

class SecurityFailure extends Failure {
  const SecurityFailure(super.message, {super.code, super.stackTrace});
}

class DeviceFailure extends Failure {
  const DeviceFailure(super.message, {super.code, super.stackTrace});
}

class StorageFailure extends Failure {
  const StorageFailure(super.message, {super.code, super.stackTrace});
}

class ValidationFailure extends Failure {
  const ValidationFailure(super.message, {super.code, super.stackTrace});
}

/// Fallos propios de Search Pro.
class SystemFailure extends Failure {
  const SystemFailure(super.message, {super.code, super.stackTrace});
}

class SearchFailure extends Failure {
  const SearchFailure(super.message, {super.code, super.stackTrace});
}

class UnexpectedFailure extends Failure {
  const UnexpectedFailure(super.message, {super.code, super.stackTrace});
}

typedef Result<T> = Either<Failure, T>;
