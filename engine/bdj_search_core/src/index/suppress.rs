//! Ventana de supresión por ruta.
//!
//! Cuando la propia aplicación copia, mueve o renombra un archivo, el cambio
//! llega al índice **dos veces**: una porque la aplicación lo anuncia —y así el
//! resultado se ve al instante, sin esperar a nadie— y otra unos milisegundos
//! después por el diario del sistema de archivos, que no distingue quién lo hizo.
//!
//! Procesar las dos produce entradas duplicadas o, peor, una lápida sobre la
//! entrada que se acababa de crear. La reconciliación es esta: al aplicar un
//! cambio anunciado por la aplicación se abre una ventana de tiempo sobre esa
//! ruta, y todo lo que llegue del sistema de archivos para la misma ruta dentro
//! de esa ventana se descarta.
//!
//! No se cierra la ventana al primer acierto porque una sola copia genera
//! varios registros —creación, extensión de datos, cierre—, y cerrar tras el
//! primero dejaría pasar los siguientes.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Cuánto se ignora una ruta tras anunciarla.
///
/// El diario USN se sondea cada 500 ms y `ReadDirectoryChangesW` entrega en
/// ráfagas: tres segundos cubren con holgura el retardo entre la escritura y su
/// notificación, sin que una ruta quede ciega el tiempo suficiente para perderse
/// un cambio real hecho por otro programa.
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(3);

/// Tope de rutas vigiladas a la vez.
///
/// Copiar diez mil archivos anuncia diez mil rutas. Sin tope, una copia grande
/// dejaría en memoria un mapa proporcional a la copia.
const MAX_ENTRIES: usize = 50_000;

/// Rutas cuyos eventos del sistema de archivos hay que ignorar durante un rato.
pub struct SuppressionWindow {
    entries: HashMap<String, Instant>,
    window: Duration,
}

impl Default for SuppressionWindow {
    fn default() -> Self {
        Self::new(DEFAULT_WINDOW)
    }
}

/// Normaliza una ruta para compararla: sin distinguir mayúsculas, con barras
/// unificadas y sin separador final.
///
/// `C:\Musica\Kick.wav`, `c:/musica/kick.wav` y `C:\Musica\Kick.wav\` son la
/// misma ruta para Windows y para macOS, y tienen que serlo también aquí.
pub fn normalize_path(path: &str) -> String {
    let unified: String = path
        .chars()
        .map(|c| if c == '/' { '\\' } else { c })
        .collect();
    let trimmed = unified.trim_end_matches('\\');
    let base = if trimmed.is_empty() { &unified } else { trimmed };
    base.to_lowercase()
}

impl SuppressionWindow {
    pub fn new(window: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            window,
        }
    }

    /// Ignora los eventos de esta ruta durante la ventana.
    pub fn suppress(&mut self, path: &str) {
        self.suppress_at(path, Instant::now());
    }

    pub fn suppress_at(&mut self, path: &str, now: Instant) {
        if self.entries.len() >= MAX_ENTRIES {
            self.purge_at(now);
        }
        if self.entries.len() >= MAX_ENTRIES {
            // Sigue lleno tras la limpieza: se prefiere procesar dos veces un
            // cambio a quedarse sin memoria.
            return;
        }
        self.entries.insert(normalize_path(path), now + self.window);
    }

    /// ¿Hay que descartar este evento?
    pub fn is_suppressed(&self, path: &str) -> bool {
        self.is_suppressed_at(path, Instant::now())
    }

    pub fn is_suppressed_at(&self, path: &str, now: Instant) -> bool {
        match self.entries.get(&normalize_path(path)) {
            Some(&until) => now < until,
            None => false,
        }
    }

    /// Retira las rutas cuya ventana ya expiró.
    pub fn purge(&mut self) {
        self.purge_at(Instant::now());
    }

    pub fn purge_at(&mut self, now: Instant) {
        self.entries.retain(|_, &mut until| now < until);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn una_ruta_anunciada_se_ignora_durante_la_ventana() {
        let mut w = SuppressionWindow::new(Duration::from_secs(3));
        let t0 = Instant::now();
        w.suppress_at("C:\\Musica\\Kick.wav", t0);
        assert!(w.is_suppressed_at("C:\\Musica\\Kick.wav", t0));
        assert!(w.is_suppressed_at("C:\\Musica\\Kick.wav", t0 + Duration::from_secs(2)));
    }

    #[test]
    fn la_ventana_expira() {
        let mut w = SuppressionWindow::new(Duration::from_secs(3));
        let t0 = Instant::now();
        w.suppress_at("C:\\Musica\\Kick.wav", t0);
        assert!(!w.is_suppressed_at("C:\\Musica\\Kick.wav", t0 + Duration::from_secs(4)));
    }

    #[test]
    fn no_se_cierra_al_primer_acierto() {
        // Una sola copia genera varios registros en el diario; los tres deben
        // descartarse.
        let mut w = SuppressionWindow::new(Duration::from_secs(3));
        let t0 = Instant::now();
        w.suppress_at("C:\\a.wav", t0);
        for _ in 0..3 {
            assert!(w.is_suppressed_at("C:\\a.wav", t0 + Duration::from_millis(100)));
        }
    }

    #[test]
    fn la_comparacion_ignora_mayusculas_barras_y_separador_final() {
        let mut w = SuppressionWindow::default();
        let t0 = Instant::now();
        w.suppress_at("C:\\Musica\\Sets\\", t0);
        assert!(w.is_suppressed_at("c:/musica/sets", t0));
        assert!(w.is_suppressed_at("C:\\MUSICA\\SETS", t0));
    }

    #[test]
    fn otra_ruta_no_se_ve_afectada() {
        let mut w = SuppressionWindow::default();
        let t0 = Instant::now();
        w.suppress_at("C:\\a.wav", t0);
        assert!(!w.is_suppressed_at("C:\\b.wav", t0));
    }

    #[test]
    fn la_limpieza_retira_lo_caducado() {
        let mut w = SuppressionWindow::new(Duration::from_secs(1));
        let t0 = Instant::now();
        w.suppress_at("C:\\a.wav", t0);
        w.suppress_at("C:\\b.wav", t0);
        assert_eq!(w.len(), 2);
        w.purge_at(t0 + Duration::from_secs(2));
        assert!(w.is_empty());
    }

    #[test]
    fn una_copia_enorme_no_agota_la_memoria() {
        let mut w = SuppressionWindow::new(Duration::from_millis(1));
        let t0 = Instant::now();
        for i in 0..(MAX_ENTRIES + 5_000) {
            w.suppress_at(&format!("C:\\a\\{i}.wav"), t0);
        }
        assert!(w.len() <= MAX_ENTRIES);
    }
}
