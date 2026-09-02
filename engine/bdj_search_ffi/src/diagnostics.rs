//! Por qué el motor no ha podido abrir el índice.
//!
//! La aplicación mostraba «Cargando motor…» ante *cualquier* fallo, y lo
//! reintentaba cada segundo indefinidamente. No estaba cargando nada: estaba
//! fallando en bucle, y el usuario no tenía forma de saber si faltaba el
//! servicio, si el índice era de un formato antiguo o si era un problema de
//! permisos.
//!
//! Este módulo separa esas causas y les da un texto que se pueda leer.

use std::path::Path;

/// Motivo por el que el índice no está disponible.
///
/// Se expone como número entero para que la interfaz decida el texto y el
/// icono, sin depender de traducir cadenas que vienen del motor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexProblem {
    /// Todo correcto.
    None = 0,
    /// El archivo no existe: el servicio no ha publicado nada todavía.
    NotFound = 1,
    /// Existe pero es de una versión anterior del formato. El servicio lo
    /// reconstruirá; hasta entonces no se puede usar.
    OldFormat = 2,
    /// Existe pero está dañado o incompleto.
    Corrupt = 3,
    /// No hay permiso para leerlo.
    Denied = 4,
    /// Cualquier otro fallo del sistema.
    Unknown = 5,
}

impl IndexProblem {
    pub fn code(self) -> u8 {
        self as u8
    }

    /// Texto para la barra de estado, en el idioma de la aplicación.
    pub fn message(self, path: &Path) -> String {
        match self {
            IndexProblem::None => "Índice listo".to_string(),
            IndexProblem::NotFound => format!(
                "No hay índice todavía. Comprueba que el servicio de indexado esté en marcha \
                 (se esperaba en {}).",
                path.display()
            ),
            IndexProblem::OldFormat => {
                "El índice es de una versión anterior. El servicio lo está reconstruyendo; \
                 el primer arranque tarda unos minutos."
                    .to_string()
            }
            IndexProblem::Corrupt => {
                "El índice está dañado. El servicio lo reconstruirá en el próximo ciclo."
                    .to_string()
            }
            IndexProblem::Denied => format!(
                "Sin permiso para leer el índice en {}. Revisa los permisos de la carpeta.",
                path.display()
            ),
            IndexProblem::Unknown => "No se pudo abrir el índice.".to_string(),
        }
    }
}

/// Clasifica un fallo al abrir el índice.
pub fn classify(path: &Path, detail: &str) -> IndexProblem {
    if !path.exists() {
        return IndexProblem::NotFound;
    }
    // `ViewError` se formatea con su `Debug`, así que estos nombres son los del
    // propio enumerado del motor.
    if detail.contains("InvalidVersion") {
        return IndexProblem::OldFormat;
    }
    if detail.contains("InvalidMagic")
        || detail.contains("CorruptedChecksum")
        || detail.contains("BufferTooSmall")
        || detail.contains("SectionOutOfBounds")
        || detail.contains("UnalignedSection")
    {
        return IndexProblem::Corrupt;
    }
    // Comprobar el permiso de lectura de verdad, y no adivinarlo por el texto
    // del error, que depende del idioma del sistema.
    match std::fs::File::open(path) {
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => IndexProblem::Denied,
        _ => IndexProblem::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn un_archivo_que_no_existe_se_reconoce() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("no-existe.bdjx");
        assert_eq!(classify(&p, "lo que sea"), IndexProblem::NotFound);
        assert!(IndexProblem::NotFound.message(&p).contains("servicio"));
    }

    #[test]
    fn un_formato_antiguo_se_reconoce() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("viejo.bdjx");
        std::fs::write(&p, b"BDJXIDX\0").unwrap();
        assert_eq!(
            classify(&p, "Formato de índice inválido: InvalidVersion(1)"),
            IndexProblem::OldFormat
        );
    }

    #[test]
    fn un_indice_danado_se_reconoce() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("roto.bdjx");
        std::fs::write(&p, b"basura").unwrap();
        assert_eq!(
            classify(&p, "Formato de índice inválido: InvalidMagic"),
            IndexProblem::Corrupt
        );
    }

    #[test]
    fn cada_problema_tiene_un_texto_distinto() {
        let p = Path::new("C:\\ProgramData\\BDJ Studio\\Search Pro\\index.bdjx");
        let textos: Vec<String> = [
            IndexProblem::None,
            IndexProblem::NotFound,
            IndexProblem::OldFormat,
            IndexProblem::Corrupt,
            IndexProblem::Denied,
            IndexProblem::Unknown,
        ]
        .iter()
        .map(|p2| p2.message(p))
        .collect();
        let mut unicos = textos.clone();
        unicos.sort();
        unicos.dedup();
        assert_eq!(unicos.len(), textos.len(), "ningún mensaje debe repetirse");
    }
}
