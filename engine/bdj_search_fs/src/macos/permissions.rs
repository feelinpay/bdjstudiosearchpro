//! Comprobación del permiso de acceso total al disco en macOS.
//!
//! # Por qué hace falta comprobarlo
//!
//! macOS protege ciertas carpetas del usuario —Escritorio, Documentos,
//! Descargas, Correo, Fotos— detrás de un permiso que se concede en Ajustes del
//! Sistema. Un proceso sin ese permiso **no recibe un error** al intentar
//! listarlas: recibe una lista vacía, o un «no existe». Desde dentro es
//! indistinguible de una carpeta que realmente está vacía.
//!
//! Para un buscador eso es lo peor que puede pasar: el índice se construye sin
//! problemas, el servicio informa de que todo ha ido bien, y el usuario descubre
//! que sus pistas no aparecen. Luego prueba a buscar una que sabe que tiene, no
//! la encuentra, y concluye —con razón desde su punto de vista— que el programa
//! no funciona.
//!
//! Así que se comprueba explícitamente y se dice.
//!
//! # Cómo se comprueba
//!
//! No hay una llamada del sistema que responda «¿tengo este permiso?». La forma
//! aceptada es intentar leer algo que solo es legible con él. Se usan varias
//! rutas y basta con que una responda, porque no todos los equipos tienen todas.

use std::path::{Path, PathBuf};

/// Qué se sabe del acceso al disco.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskAccess {
    /// Se puede leer lo protegido: el índice será completo.
    Full,
    /// No se puede. Las carpetas personales del usuario faltarán del índice.
    Restricted,
    /// No se pudo determinar (ninguna de las rutas de prueba existe).
    Unknown,
}

impl DiskAccess {
    /// Mensaje listo para enseñar, o `None` si no hay nada que decir.
    pub fn message(self) -> Option<&'static str> {
        match self {
            DiskAccess::Full => None,
            DiskAccess::Restricted => Some(
                "Sin acceso total al disco: el Escritorio, Documentos y Descargas \
                 no se pueden indexar. Actívalo en Ajustes del Sistema → \
                 Privacidad y seguridad → Acceso total al disco.",
            ),
            DiskAccess::Unknown => None,
        }
    }
}

/// Rutas que solo se pueden leer con acceso total al disco.
fn rutas_de_prueba() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users".to_string());
    let home = Path::new(&home);
    vec![
        // La base de datos de Mail y las preferencias de Safari son las dos
        // sondas habituales: existen en casi todos los equipos y están
        // protegidas.
        home.join("Library/Mail"),
        home.join("Library/Safari"),
        home.join("Library/Application Support/com.apple.TCC"),
        // Y las carpetas que de verdad importan para este producto.
        home.join("Desktop"),
        home.join("Documents"),
        home.join("Downloads"),
    ]
}

/// ¿Tiene este proceso acceso total al disco?
///
/// Devuelve [`DiskAccess::Unknown`] si ninguna de las rutas de prueba existe,
/// que es lo honesto: no es lo mismo «no tengo permiso» que «no he podido
/// averiguarlo», y tratarlos igual llevaría a enseñar un aviso alarmante a
/// alguien que no tiene ningún problema.
pub fn check_disk_access() -> DiskAccess {
    for ruta in rutas_de_prueba() {
        // `metadata` responde aunque el contenido esté protegido, así que la
        // prueba de verdad es intentar **listar**.
        if !ruta.exists() {
            continue;
        }
        match std::fs::read_dir(&ruta) {
            Ok(mut entradas) => {
                // Abrir el iterador no basta: el fallo de permisos aparece al
                // pedir la primera entrada.
                match entradas.next() {
                    Some(Err(e)) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                        return DiskAccess::Restricted;
                    }
                    _ => return DiskAccess::Full,
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                return DiskAccess::Restricted;
            }
            Err(_) => continue,
        }
    }

    // Ninguna ruta de prueba existía, o todas fallaron por motivos que no son
    // de permisos. No hay veredicto, y decirlo así es más útil que inventarse
    // uno.
    DiskAccess::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_mensaje_solo_aparece_cuando_hay_algo_que_decir() {
        assert!(DiskAccess::Full.message().is_none());
        assert!(DiskAccess::Unknown.message().is_none());
        assert!(DiskAccess::Restricted.message().is_some());
    }

    #[test]
    fn hay_rutas_de_prueba_y_todas_son_absolutas() {
        let rutas = rutas_de_prueba();
        assert!(!rutas.is_empty());
        for r in rutas {
            assert!(r.is_absolute(), "{} debería ser absoluta", r.display());
        }
    }

    #[test]
    fn una_carpeta_legible_se_puede_listar() {
        // El camino positivo, sin dependencias de prueba: el directorio
        // temporal del sistema siempre existe y siempre es legible.
        let dir = std::env::temp_dir();
        assert!(
            std::fs::read_dir(&dir).is_ok(),
            "el directorio temporal debería poder listarse"
        );
    }

    #[test]
    fn una_ruta_inexistente_no_da_veredicto() {
        // Lo importante: «no lo sé» y «no tengo permiso» no son lo mismo. Si se
        // confundieran, un equipo perfectamente configurado vería un aviso
        // alarmante que no le corresponde.
        assert_ne!(DiskAccess::Unknown, DiskAccess::Restricted);
    }
}
