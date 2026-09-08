//! Lo que ninguna operación puede tocar, venga la orden de donde venga.
//!
//! # Por qué existe
//!
//! Un usuario informó de esto: «No se pudo enviar «C:\» a la papelera». La
//! aplicación había llegado a **pedirle al sistema que mandara la raíz del disco
//! a la papelera**. Salvó la situación que la biblioteca de papelera se negó por
//! su cuenta, con un error interno (`TargetedRoot`) que además no significa nada
//! para quien lo lee.
//!
//! Depender de eso es inaceptable. El origen del fallo estaba más arriba —una
//! fila que resolvía a la raíz de un volumen en vez de al archivo elegido— y
//! ese fallo concreto está corregido, pero la lección es otra: **una operación
//! destructiva no puede fiarse de que quien la llama le pase algo razonable**.
//! Entre la selección de una fila y el borrado de un archivo hay una tabla
//! virtualizada, una caché de páginas, dos espacios de identificadores y un
//! índice que se renumera solo. Cualquiera de esas piezas puede equivocarse un
//! día; ninguna debería poder equivocarse *hasta el punto de borrar un disco*.
//!
//! Así que aquí se rechaza lo que no se puede tocar, antes de intentarlo, y con
//! un mensaje que se entiende.
//!
//! # Qué se protege
//!
//! - La raíz de un volumen: `C:\`, `D:/`, `/`, `/Volumes/Musica`.
//! - El directorio personal del usuario y las carpetas del sistema que macOS y
//!   Windows esperan encontrar en su sitio.
//!
//! Lo que **no** se protege, a propósito: cualquier otra cosa. Esto no es un
//! filtro de sentido común ni una lista de carpetas «importantes». Es una barrera
//! contra un error de programación, no contra una decisión del usuario.

use std::path::{Component, Path};

/// Por qué no se puede operar sobre una ruta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Forbidden {
    /// Es la raíz de un volumen o del sistema de archivos.
    VolumeRoot,
    /// Es el directorio personal del usuario.
    HomeDirectory,
    /// Es una carpeta que el sistema operativo necesita donde está.
    SystemDirectory,
    /// La ruta está vacía.
    Empty,
}

impl Forbidden {
    /// Mensaje para el usuario. Dice qué pasa, no qué error interno hubo.
    pub fn message(&self, path: &Path) -> String {
        match self {
            Forbidden::VolumeRoot => format!(
                "«{}» es la raíz de una unidad y no se puede mover ni borrar.",
                path.display()
            ),
            Forbidden::HomeDirectory => format!(
                "«{}» es tu carpeta personal y no se puede mover ni borrar.",
                path.display()
            ),
            Forbidden::SystemDirectory => format!(
                "«{}» es una carpeta del sistema y no se puede mover ni borrar.",
                path.display()
            ),
            Forbidden::Empty => "No se indicó ninguna ruta.".to_string(),
        }
    }
}

/// Normaliza para comparar: sin separador final, en minúsculas.
fn normalizar(p: &Path) -> String {
    let s = p.to_string_lossy().replace('/', "\\");
    let s = s.trim_end_matches('\\');
    if s.is_empty() {
        "\\".to_string()
    } else {
        s.to_ascii_lowercase()
    }
}

/// ¿Es la raíz de un volumen o del sistema de archivos?
///
/// Cubre las tres formas que puede tomar: `C:\` en Windows, `/` en Unix, y un
/// punto de montaje directo de `/Volumes` en macOS —un disco externo montado ahí
/// es tan raíz como `C:\`, aunque tenga dos componentes de ruta—.
pub fn is_volume_root(path: &Path) -> bool {
    let texto = path.to_string_lossy();
    let recortado = texto.trim_end_matches(['\\', '/']);

    // `C:` o `C:\` — sin nada detrás.
    let bytes = recortado.as_bytes();
    if bytes.len() == 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return true;
    }

    // `/` o `\` a secas: al recortar no queda nada.
    if recortado.is_empty() && !texto.is_empty() {
        return true;
    }

    // Recuento de componentes reales, ignorando prefijos y raíces.
    let normales: Vec<_> = path
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .collect();

    if normales.is_empty() {
        // Solo prefijo y/o raíz: `C:\`, `/`, `\\servidor\recurso`.
        return path.components().next().is_some();
    }

    // Puntos de montaje: `/Volumes/X` y `/mnt/X` son raíces de un volumen.
    if normales.len() == 2 {
        let primero = normales[0].as_os_str().to_string_lossy().to_ascii_lowercase();
        if primero == "volumes" || primero == "mnt" || primero == "media" {
            return true;
        }
    }

    false
}

/// Carpetas que el sistema espera encontrar donde están.
fn es_directorio_de_sistema(normalizada: &str) -> bool {
    const PROTEGIDAS: [&str; 12] = [
        "c:\\windows",
        "c:\\program files",
        "c:\\program files (x86)",
        "c:\\programdata",
        "c:\\users",
        "\\system",
        "\\library",
        "\\applications",
        "\\usr",
        "\\bin",
        "\\etc",
        "\\var",
    ];
    PROTEGIDAS.contains(&normalizada)
}

/// Directorio personal del usuario, si se puede averiguar.
fn home() -> Option<String> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var(var).ok().map(|h| normalizar(Path::new(&h)))
}

/// ¿Se puede operar sobre esta ruta?
///
/// Devuelve `Err` con el motivo cuando no. Es lo primero que hace cualquier
/// operación que mueva o borre.
pub fn check_operable(path: &Path) -> Result<(), Forbidden> {
    if path.as_os_str().is_empty() {
        return Err(Forbidden::Empty);
    }
    if is_volume_root(path) {
        return Err(Forbidden::VolumeRoot);
    }

    let n = normalizar(path);
    if let Some(h) = home()
        && n == h
    {
        return Err(Forbidden::HomeDirectory);
    }
    if es_directorio_de_sistema(&n) {
        return Err(Forbidden::SystemDirectory);
    }
    Ok(())
}

/// Comprueba una lista y devuelve el primer motivo de rechazo, si lo hay.
pub fn check_all(paths: &[std::path::PathBuf]) -> Result<(), String> {
    for p in paths {
        if let Err(motivo) = check_operable(p) {
            return Err(motivo.message(p));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn la_raiz_de_una_unidad_de_windows_esta_protegida() {
        // El caso exacto del informe: la aplicación llegó a pedir que «C:\»
        // fuera a la papelera.
        for r in ["C:\\", "C:", "c:\\", "D:/", "E:"] {
            assert!(
                is_volume_root(Path::new(r)),
                "«{r}» debería reconocerse como raíz de unidad"
            );
            assert_eq!(
                check_operable(Path::new(r)),
                Err(Forbidden::VolumeRoot),
                "«{r}» no puede ser operable"
            );
        }
    }

    #[test]
    fn la_raiz_de_unix_esta_protegida() {
        assert!(is_volume_root(Path::new("/")));
        assert_eq!(check_operable(Path::new("/")), Err(Forbidden::VolumeRoot));
    }

    #[test]
    fn un_disco_externo_de_macos_es_tan_raiz_como_c() {
        // `/Volumes/Musica` tiene dos componentes pero es el punto de montaje de
        // un disco entero: borrarlo es borrar el disco.
        assert!(is_volume_root(Path::new("/Volumes/Musica")));
        assert!(is_volume_root(Path::new("/Volumes/USB Cabina/")));
        // Pero lo que hay dentro es un archivo normal y corriente.
        assert!(!is_volume_root(Path::new("/Volumes/Musica/set.wav")));
        assert!(check_operable(Path::new("/Volumes/Musica/set.wav")).is_ok());
    }

    #[test]
    fn un_archivo_normal_se_puede_operar() {
        for r in [
            "C:\\Users\\David Zapata\\Desktop\\pista.wav",
            "C:\\Musica",
            "/Users/david/Music/set.wav",
            "D:\\Sets\\2026",
        ] {
            assert!(
                check_operable(Path::new(r)).is_ok(),
                "«{r}» debería poder operarse"
            );
        }
    }

    #[test]
    fn las_carpetas_del_sistema_estan_protegidas() {
        assert_eq!(
            check_operable(Path::new("C:\\Windows")),
            Err(Forbidden::SystemDirectory)
        );
        assert_eq!(
            check_operable(Path::new("C:\\Users")),
            Err(Forbidden::SystemDirectory)
        );
        // Pero la carpeta de un usuario concreto sí se puede tocar.
        assert!(check_operable(Path::new("C:\\Users\\David Zapata\\Musica")).is_ok());
    }

    #[test]
    fn una_ruta_vacia_se_rechaza() {
        assert_eq!(check_operable(Path::new("")), Err(Forbidden::Empty));
    }

    #[test]
    fn el_mensaje_dice_la_ruta_y_no_un_error_interno() {
        // «Error during a `trash` operation: TargetedRoot» no le dice nada a
        // nadie. El mensaje tiene que nombrar el archivo y el motivo.
        let m = Forbidden::VolumeRoot.message(Path::new("C:\\"));
        assert!(m.contains("C:\\"));
        assert!(m.contains("raíz"));
        assert!(!m.to_lowercase().contains("targetedroot"));
    }

    #[test]
    fn comprobar_una_lista_devuelve_el_primer_motivo() {
        let rutas = vec![
            PathBuf::from("C:\\Musica\\a.wav"),
            PathBuf::from("C:\\"),
            PathBuf::from("C:\\Musica\\b.wav"),
        ];
        let error = check_all(&rutas).unwrap_err();
        assert!(error.contains("C:\\"));

        // Y una lista sana pasa entera.
        let sanas = vec![
            PathBuf::from("C:\\Musica\\a.wav"),
            PathBuf::from("C:\\Musica\\b.wav"),
        ];
        assert!(check_all(&sanas).is_ok());
    }
}
