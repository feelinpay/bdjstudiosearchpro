/// Modos de visualización de resultados en la interfaz.
enum ResultViewMode {
  details('Detalles', 'Tabla detallada con columnas'),
  list('Lista', 'Lista simple de elementos'),
  compact('Compacta', 'Lista densa con alta concentración de filas'),
  grid('Iconos', 'Cuadrícula de tarjetas con iconos');

  final String label;
  final String description;

  const ResultViewMode(this.label, this.description);
}
