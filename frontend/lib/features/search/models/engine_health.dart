import 'package:flutter/foundation.dart';

/// Por qué el índice no está disponible.
///
/// Los códigos los define el motor en `diagnostics.rs`; aquí solo se traducen a
/// algo que la interfaz pueda pintar.
enum IndexProblem {
  none,
  notFound,
  oldFormat,
  corrupt,
  denied,
  unknown;

  static IndexProblem fromCode(int code) => switch (code) {
    0 => IndexProblem.none,
    1 => IndexProblem.notFound,
    2 => IndexProblem.oldFormat,
    3 => IndexProblem.corrupt,
    4 => IndexProblem.denied,
    _ => IndexProblem.unknown,
  };

  /// Cierto cuando la situación se resuelve sola en cuanto el servicio trabaje.
  bool get resolvesItself =>
      this == IndexProblem.notFound ||
      this == IndexProblem.oldFormat ||
      this == IndexProblem.corrupt;
}

/// Estado real del motor, tal y como lo cuenta el propio motor.
///
/// La interfaz mostraba «Cargando motor…» ante cualquier fallo y lo reintentaba
/// cada segundo para siempre. No estaba cargando nada: estaba fallando en bucle,
/// y no había forma de distinguir «falta el servicio» de «el índice es de una
/// versión anterior» o de «no hay permisos».
@immutable
class EngineHealth {
  final bool isOpen;
  final String indexPath;
  final bool fileExists;
  final int fileSize;
  final int generation;
  final int entryCount;
  final IndexProblem problem;
  final String message;

  /// Cierto mientras la primera comprobación del motor no ha devuelto nada.
  ///
  /// Es un estado *pendiente de resolver*, no un error: la interfaz no debe
  /// pintarlo en rojo, o el arranque destellaría «No se pudo abrir el índice»
  /// durante unos fotogramas antes de llegar el veredicto real.
  final bool isChecking;

  /// Marca temporal en la que el motor llegó a un veredicto por primera vez.
  ///
  /// Aunque el veredicto sea «no se pudo abrir el índice», durante los primeros
  /// segundos se sigue mostrando un aviso neutro, porque el servicio puede estar
  /// arrancando todavía. Sin este compás de espera, el primer arranque de un
  /// equipo sin índice aún destellaba un error rojo durante varios fotogramas.
  final DateTime? decidedAt;

  const EngineHealth({
    this.isOpen = false,
    this.indexPath = '',
    this.fileExists = false,
    this.fileSize = 0,
    this.generation = 0,
    this.entryCount = 0,
    this.problem = IndexProblem.unknown,
    this.message = 'Comprobando el índice…',
    this.isChecking = true,
    this.decidedAt,
  });

  static const EngineHealth checking = EngineHealth();

  /// El motor ya dio un veredicto (sea bueno o malo) y pasó el periodo de gracia.
  ///
  /// Mientras no se cumple, los fallos «se resuelven solos» (sin índice, índice
  /// antiguo, dañado) se siguen pintando como un aviso, no como un error: el
  /// servicio puede estar levantándose todavía.
  bool get isStable {
    if (isOpen) return true;
    final t = decidedAt;
    if (t == null) return false;
    return DateTime.now().difference(t) >= _gracePeriod;
  }

  /// Ventana en la que un fallo «se resuelve solo» todavía se pinta como aviso.
  static const Duration _gracePeriod = Duration(seconds: 3);

  /// Texto corto para la barra de estado.
  String get shortLabel {
    if (isOpen) return 'Índice listo · gen $generation';
    if (isChecking) return 'Arrancando el servicio de indexado…';
    if (!isStable) {
      // Aún dentro del periodo de gracia: el servicio puede estar arrancando
      // y publicando el primer índice de un momento a otro. Se pinta como un
      // aviso, no como un error.
      return switch (problem) {
        IndexProblem.notFound => 'Iniciando el índice por primera vez…',
        IndexProblem.oldFormat => 'Reconstruyendo el índice…',
        IndexProblem.corrupt => 'Reconstruyendo el índice…',
        IndexProblem.denied => 'Sin permiso para leer el índice',
        _ => 'Iniciando el servicio de búsqueda…',
      };
    }
    return switch (problem) {
      IndexProblem.notFound => 'Sin índice: el servicio no está en marcha',
      IndexProblem.oldFormat => 'Reconstruyendo el índice…',
      IndexProblem.corrupt => 'Índice dañado: se reconstruirá',
      IndexProblem.denied => 'Sin permiso para leer el índice',
      IndexProblem.none => 'Preparando el índice…',
      IndexProblem.unknown => 'No se pudo abrir el índice',
    };
  }

  EngineHealth copyWith({
    bool? isOpen,
    String? indexPath,
    bool? fileExists,
    int? fileSize,
    int? generation,
    int? entryCount,
    IndexProblem? problem,
    String? message,
    bool? isChecking,
    DateTime? decidedAt,
  }) {
    return EngineHealth(
      isOpen: isOpen ?? this.isOpen,
      indexPath: indexPath ?? this.indexPath,
      fileExists: fileExists ?? this.fileExists,
      fileSize: fileSize ?? this.fileSize,
      generation: generation ?? this.generation,
      entryCount: entryCount ?? this.entryCount,
      problem: problem ?? this.problem,
      message: message ?? this.message,
      isChecking: isChecking ?? this.isChecking,
      decidedAt: decidedAt ?? this.decidedAt,
    );
  }

  /// Explicación completa, para la ayuda emergente.
  String get detail => message;

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      other is EngineHealth &&
          runtimeType == other.runtimeType &&
          isOpen == other.isOpen &&
          indexPath == other.indexPath &&
          fileExists == other.fileExists &&
          fileSize == other.fileSize &&
          generation == other.generation &&
          entryCount == other.entryCount &&
          problem == other.problem &&
          message == other.message &&
          isChecking == other.isChecking &&
          decidedAt == other.decidedAt;

  @override
  int get hashCode => Object.hash(
    isOpen,
    indexPath,
    fileExists,
    fileSize,
    generation,
    entryCount,
    problem,
    message,
    isChecking,
    decidedAt,
  );
}
