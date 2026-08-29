import 'package:flutter/material.dart';

/// Paleta de colores oficial para BDJ Studio Search Pro (Tema Blanco de alto contraste).
/// Especificación del Plan Maestro §9.
class AppColors {
  AppColors._();

  // Superficies
  static const Color background = Color(0xFFFFFFFF); // Lienzo puro
  static const Color surface = Color(0xFFF7F8FA); // Barras, cabeceras
  static const Color surfaceAlt = Color(0xFFFCFCFD); // Filas alternas sutiles
  static const Color rowOdd = Color(0xFFFFFFFF); // Fila impar (blanco puro)
  static const Color rowEven = Color(0xFFF9F9FB); // Fila par (gris sutil)
  static const Color hover = Color(0xFFF1F3F7); // Hover de filas y botones
  static const Color selected = Color(0xFFEAE7FF); // Selección (violeta muy claro)
  static const Color border = Color(0xFFE6E8EC); // Separadores suaves de 1px
  static const Color borderStrong = Color(0xFFD3D7DE);

  // Texto (contraste accesible sobre fondo blanco)
  static const Color textPrimary = Color(0xFF14181F); // Ratio 15,8:1
  static const Color textSecondary = Color(0xFF5B6472); // Ratio 6,1:1
  static const Color textDisabled = Color(0xFF9AA2AE); // Ratio 2,8:1 (solo inactivo)

  // Acento - Violeta BDJ ajustado para fondo blanco
  static const Color primary = Color(0xFF5B4BFF); // Ratio 5,9:1 (cumple AA)
  static const Color primaryHover = Color(0xFF4A3AEE);
  static const Color primarySoft = Color(0xFFEEECFF); // Fondo de resaltado de búsqueda

  // Estados
  static const Color success = Color(0xFF0A7C42); // 5,3:1
  static const Color warning = Color(0xFF8A5A00); // 5,4:1
  static const Color error = Color(0xFFC0192B); // 6,2:1

  // Color por tipo de archivo (iconos y chips de filtro)
  static const Color kindAudio = Color(0xFF5B4BFF);
  static const Color kindVideo = Color(0xFFD1355A);
  static const Color kindImage = Color(0xFF0A7C42);
  static const Color kindDocument = Color(0xFF2563A8);
  static const Color kindArchive = Color(0xFF8A5A00);
  static const Color kindFolder = Color(0xFF6B7280);
  static const Color kindApp = Color(0xFF7A3E9D);
}
