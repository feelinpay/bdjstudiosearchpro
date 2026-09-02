import 'package:flutter/foundation.dart';

/// Qué filas están seleccionadas, sin materializar millones de enteros.
///
/// «Seleccionar todo» sobre un resultado de nueve millones de archivos no puede
/// construir un conjunto de nueve millones de enteros: son treinta y seis megas
/// para pintar treinta filas. Por eso el modelo tiene dos modos:
///
/// - normal: `indices` son las filas seleccionadas;
/// - invertido: `indices` son las filas **excluidas**, y el resto está
///   seleccionado.
///
/// «Seleccionar todo» es entonces cambiar un booleano, e ir deseleccionando a
/// mano solo cuesta lo que se quita.
@immutable
class Selection {
  /// Cierto cuando `indices` significa «todo menos estos».
  final bool inverted;
  final Set<int> indices;

  /// Fila desde la que se mide un rango con Mayús.
  final int anchor;

  /// Fila con el foco: la que se abre con Intro y desde la que se mueve.
  final int cursor;

  /// Número total de filas del resultado. Necesario para contar en modo
  /// invertido.
  final int total;

  const Selection({
    this.inverted = false,
    this.indices = const {},
    this.anchor = 0,
    this.cursor = 0,
    this.total = 0,
  });

  static const Selection empty = Selection();

  bool contains(int index) =>
      inverted ? !indices.contains(index) : indices.contains(index);

  int get count => inverted ? (total - indices.length).clamp(0, total) : indices.length;

  bool get isEmpty => count == 0;
  bool get isNotEmpty => count > 0;

  Selection withTotal(int newTotal) => _copy(total: newTotal);

  /// Una sola fila, que además pasa a ser el ancla.
  Selection single(int index) => Selection(
    inverted: false,
    indices: {index},
    anchor: index,
    cursor: index,
    total: total,
  );

  /// Añade o quita una fila sin tocar el resto. Es Ctrl/Cmd + clic.
  Selection toggle(int index) {
    final next = Set<int>.from(indices);
    if (next.contains(index)) {
      next.remove(index);
    } else {
      next.add(index);
    }
    return Selection(
      inverted: inverted,
      indices: next,
      anchor: index,
      cursor: index,
      total: total,
    );
  }

  /// Todo lo que hay entre el ancla y `index`. Es Mayús + clic.
  ///
  /// El ancla no se mueve: así se puede ir ampliando y reduciendo el rango sin
  /// perder el punto de partida, que es como se comporta cualquier explorador.
  Selection range(int index) {
    final desde = anchor <= index ? anchor : index;
    final hasta = anchor <= index ? index : anchor;
    final next = <int>{};
    for (var i = desde; i <= hasta; i++) {
      next.add(i);
    }
    return Selection(
      inverted: false,
      indices: next,
      anchor: anchor,
      cursor: index,
      total: total,
    );
  }

  /// Todas las filas del resultado, incluidas las que aún no se han pedido.
  Selection all(int newTotal) => Selection(
    inverted: true,
    indices: const {},
    anchor: anchor,
    cursor: cursor,
    total: newTotal,
  );

  Selection clear() => Selection(
    inverted: false,
    indices: const {},
    anchor: anchor,
    cursor: cursor,
    total: total,
  );

  /// Invierte la selección. Sobre millones de filas cuesta lo mismo que sobre
  /// tres.
  Selection invert() => Selection(
    inverted: !inverted,
    indices: indices,
    anchor: anchor,
    cursor: cursor,
    total: total,
  );

  Selection moveCursor(int index) => Selection(
    inverted: false,
    indices: {index},
    anchor: index,
    cursor: index,
    total: total,
  );

  /// Extiende la selección con el teclado, conservando el ancla.
  Selection extendCursor(int index) => range(index);

  /// Los índices concretos, como mucho `max`.
  ///
  /// Una operación de archivo necesita la lista de verdad. El tope evita que
  /// «seleccionar todo» sobre nueve millones de resultados intente construir
  /// nueve millones de rutas; quien llama comprueba `count` y avisa si se pasa.
  List<int> resolve({int max = 10000}) {
    if (!inverted) {
      final lista = indices.toList()..sort();
      return lista.length <= max ? lista : lista.sublist(0, max);
    }
    final out = <int>[];
    for (var i = 0; i < total && out.length < max; i++) {
      if (!indices.contains(i)) out.add(i);
    }
    return out;
  }

  Selection _copy({bool? inverted, Set<int>? indices, int? anchor, int? cursor, int? total}) =>
      Selection(
        inverted: inverted ?? this.inverted,
        indices: indices ?? this.indices,
        anchor: anchor ?? this.anchor,
        cursor: cursor ?? this.cursor,
        total: total ?? this.total,
      );

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      other is Selection &&
          runtimeType == other.runtimeType &&
          inverted == other.inverted &&
          setEquals(indices, other.indices) &&
          anchor == other.anchor &&
          cursor == other.cursor &&
          total == other.total;

  @override
  int get hashCode =>
      Object.hash(inverted, indices.length, anchor, cursor, total);
}
