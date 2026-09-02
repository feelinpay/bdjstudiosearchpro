use std::path::{Path, PathBuf};

/// Devuelve una ruta libre añadiendo « (2)», « (3)»… antes de la extensión.
///
/// Es lo que hace el Explorador y lo que espera cualquiera: `pista.wav` se
/// convierte en `pista (2).wav`, no en `pista.wav (2)`. Las carpetas no llevan
/// extensión, así que el sufijo va al final.
///
/// Devuelve `None` si tras muchos intentos sigue sin haber hueco: es preferible
/// fallar a entrar en un bucle infinito.
pub fn unique_path(desired: &Path) -> Option<PathBuf> {
    if !desired.exists() {
        return Some(desired.to_path_buf());
    }

    let parent = desired.parent().unwrap_or(Path::new("."));
    let file_name = desired.file_name()?.to_string_lossy().to_string();

    // Solo se considera extensión lo que va tras el último punto, y nunca un
    // nombre que empieza por punto («.gitignore» no tiene extensión).
    let (stem, ext) = match file_name.rfind('.') {
        Some(pos) if pos > 0 => (&file_name[..pos], Some(&file_name[pos + 1..])),
        _ => (file_name.as_str(), None),
    };

    for n in 2..10_000u32 {
        let candidate_name = match ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        let candidate = parent.join(candidate_name);
        if !candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Comprueba que un nombre de archivo es utilizable en Windows y macOS.
///
/// Se valida aquí y no en la interfaz porque la comprobación también protege
/// al motor: un nombre con `..` o con una barra convertiría un renombrado en un
/// movimiento a otra carpeta.
pub fn validate_file_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("El nombre no puede estar vacío.".into());
    }
    if name == "." || name == ".." {
        return Err("Ese nombre está reservado por el sistema.".into());
    }
    if name.len() > 255 {
        return Err("El nombre supera los 255 caracteres.".into());
    }
    if let Some(c) = name
        .chars()
        .find(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
    {
        return Err(format!("El nombre no puede contener «{c}»."));
    }
    if name.chars().any(|c| (c as u32) < 0x20) {
        return Err("El nombre contiene caracteres de control.".into());
    }
    if name.ends_with(' ') || name.ends_with('.') {
        return Err("El nombre no puede terminar en espacio ni en punto.".into());
    }

    // Nombres de dispositivo heredados de MS-DOS: Windows los rechaza incluso
    // con extensión.
    const RESERVADOS: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    if RESERVADOS.contains(&stem.as_str()) {
        return Err(format!("«{stem}» es un nombre reservado por Windows."));
    }

    Ok(())
}

/// Cierto si `child` está dentro de `parent` (o es el mismo).
///
/// Mover una carpeta dentro de sí misma crearía un bucle infinito de copia;
/// esta comprobación es lo que lo impide.
pub fn is_inside(child: &Path, parent: &Path) -> bool {
    let c = child.canonicalize().unwrap_or_else(|_| child.to_path_buf());
    let p = parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf());
    c.starts_with(&p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn el_sufijo_va_antes_de_la_extension() {
        let dir = tempdir().unwrap();
        let f = dir.path().join("pista.wav");
        std::fs::write(&f, b"x").unwrap();
        let libre = unique_path(&f).unwrap();
        assert_eq!(libre.file_name().unwrap(), "pista (2).wav");
    }

    #[test]
    fn el_sufijo_sube_hasta_encontrar_hueco() {
        let dir = tempdir().unwrap();
        for n in ["pista.wav", "pista (2).wav", "pista (3).wav"] {
            std::fs::write(dir.path().join(n), b"x").unwrap();
        }
        let libre = unique_path(&dir.path().join("pista.wav")).unwrap();
        assert_eq!(libre.file_name().unwrap(), "pista (4).wav");
    }

    #[test]
    fn una_carpeta_recibe_el_sufijo_al_final() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("Sesion");
        std::fs::create_dir(&sub).unwrap();
        let libre = unique_path(&sub).unwrap();
        assert_eq!(libre.file_name().unwrap(), "Sesion (2)");
    }

    #[test]
    fn un_nombre_oculto_no_tiene_extension() {
        let dir = tempdir().unwrap();
        let f = dir.path().join(".gitignore");
        std::fs::write(&f, b"x").unwrap();
        let libre = unique_path(&f).unwrap();
        assert_eq!(libre.file_name().unwrap(), ".gitignore (2)");
    }

    #[test]
    fn una_ruta_libre_se_devuelve_tal_cual() {
        let dir = tempdir().unwrap();
        let f = dir.path().join("nuevo.wav");
        assert_eq!(unique_path(&f).unwrap(), f);
    }

    #[test]
    fn nombres_invalidos() {
        assert!(validate_file_name("").is_err());
        assert!(validate_file_name("..").is_err());
        assert!(validate_file_name("a/b.wav").is_err());
        assert!(validate_file_name("a:b.wav").is_err());
        assert!(validate_file_name("CON").is_err());
        assert!(validate_file_name("con.txt").is_err());
        assert!(validate_file_name("termina en punto.").is_err());
        assert!(validate_file_name("termina en espacio ").is_err());
        assert!(validate_file_name("Michael Jackson - Billie Jean.mp3").is_ok());
        assert!(validate_file_name("Canción ñ 日本語.wav").is_ok());
    }

    #[test]
    fn detecta_una_carpeta_dentro_de_si_misma() {
        let dir = tempdir().unwrap();
        let padre = dir.path().join("Sets");
        let hijo = padre.join("2026");
        std::fs::create_dir_all(&hijo).unwrap();
        assert!(is_inside(&hijo, &padre));
        assert!(is_inside(&padre, &padre));
        assert!(!is_inside(&padre, &hijo));
    }
}
