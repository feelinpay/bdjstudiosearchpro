abstract class Failure {
  final String message;
  final String? code;

  const Failure(this.message, {this.code});

  @override
  String toString() => '$runtimeType(message: $message, code: $code)';
}

class SystemFailure extends Failure {
  const SystemFailure(super.message, {super.code});
}

class LicenseFailure extends Failure {
  const LicenseFailure(super.message, {super.code});
}

class SearchFailure extends Failure {
  const SearchFailure(super.message, {super.code});
}
