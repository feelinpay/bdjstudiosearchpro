//! Papelera y restauración nativas de macOS.
//!
//! La crate `trash` puede **borrar** en macOS, pero no expone listar ni
//! restaurar: su módulo `os_limited` no existe ahí. Para que macOS tenga el
//! mismo alcance que Windows no vale avisar de la limitación; hay que hacer la
//! papelera uno mismo.
//!
//! El plan: mover el elemento a la papelera del sistema —`~/.Trash` o
//! `.Trashes/<uid>` del volumen— y guardar la ruta original en un atributo
//! extendido (`com.bdjstudio.original-path`). Restaurar lee ese atributo y,
//! si no está, busca por nombre. Es el equivalente a lo que `os_limited` hace
//! con el `TrashItem` de Windows.

use std::ffi::{CString, OsString};
use std::os::raw::c_void;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

pub const ORIGINAL_PATH_XATTR: &str = "com.bdjstudio.original-path";

fn uid_string() -> String {
    unsafe { libc::getuid().to_string() }
}

/// Copia un árbol con un copiador mínimo y conservando la fecha de la copia.
///
/// Solo se usa cuando la papelera del volumen no existe y morirse a la de casa
/// cruza volumenes: ahí `rename` devuelve EXDEV y hay que copiar y borrar.
fn copy_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    let md = std::fs::symlink_metadata(src).map_err(|e| e.to_string())?;
    if md.is_dir() {
        std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
        let rd = std::fs::read_dir(src).map_err(|e| e.to_string())?;
        for entry in rd.flatten() {
            copy_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(src, dst).map_err(|e| e.to_string())?;
        if let Ok(md) = std::fs::metadata(src)
            && let Ok(mtime) = md.modified()
            && let Ok(f) = std::fs::File::options().write(true).open(dst)
        {
            let _ = f.set_modified(mtime);
        }
        Ok(())
    }
}

/// Guarda en `item` la ruta original. Fallos aquí no son mortales: la propia
/// aplicación ya conserva el original en su recibo; el atributo es la red de
/// seguridad para una restauración que llegue después.
fn set_original_path(item: &Path, original: &Path) {
    let Ok(p) = CString::new(item.as_os_str().as_bytes()) else {
        return;
    };
    let Ok(name) = CString::new(ORIGINAL_PATH_XATTR) else {
        return;
    };
    let value = original.as_os_str().as_bytes();
    unsafe {
        libc::setxattr(
            p.as_ptr(),
            name.as_ptr(),
            value.as_ptr() as *const c_void,
            value.len(),
            0,
            0,
        );
    }
}

fn get_original_path(item: &Path) -> Option<PathBuf> {
    let p = CString::new(item.as_os_str().as_bytes()).ok()?;
    let name = CString::new(ORIGINAL_PATH_XATTR).ok()?;
    let mut buf = [0u8; 4096];
    let n = unsafe {
        libc::getxattr(
            p.as_ptr(),
            name.as_ptr(),
            buf.as_mut_ptr() as *mut c_void,
            buf.len(),
            0,
            0,
        )
    };
    if n <= 0 {
        return None;
    }
    Some(PathBuf::from(OsString::from_vec(buf[..n as usize].to_vec())))
}

/// Papeleras donde puede estar un elemento: la de casa y las de los volumenes
/// externos montados en ese momento.
fn search_trash_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(PathBuf::from(home).join(".Trash"));
    }
    if let Ok(entries) = std::fs::read_dir("/Volumes") {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() && !p.is_symlink() {
                dirs.push(p.join(".Trashes").join(uid_string()));
            }
        }
    }
    dirs
}

/// Envía `source` a la papelera y devuelve dónde quedó.
///
/// Prioriza la papelera del propio volumen (`.Trashes/<uid>`) —el mover es
/// instantáneo— y cae a `~/.Trash` cuando la del volumen no existe. Si ambas
/// están en volumenes distintos, copia el árbol y retira el original.
pub fn trash_path(source: &Path) -> Result<PathBuf, String> {
    let name = source
        .file_name()
        .ok_or("El elemento no tiene nombre de archivo.")?;

    let mut candidate_dirs: Vec<PathBuf> = Vec::new();
    if let Ok(stripped) = source.strip_prefix("/Volumes")
        && let Some(vol_name) = stripped.iter().next()
    {
        candidate_dirs.push(PathBuf::from("/Volumes").join(vol_name).join(".Trashes").join(uid_string()));
    }
    if let Ok(home) = std::env::var("HOME") {
        candidate_dirs.push(PathBuf::from(home).join(".Trash"));
    }

    let base_name = name.to_string_lossy().into_owned();
    let mut last_err = String::from("no hay papelera disponible.");

    for dir in &candidate_dirs {
        if std::fs::create_dir_all(dir).is_err() {
            continue;
        }
        // Nombre libre estilo Finder: foo.ext pasa a "foo 2.ext".
        let mut target = dir.join(name);
        let mut i = 2usize;
        while target.exists() {
            let nombre = match base_name.rsplit_once('.') {
                Some((stem, ext)) => format!("{stem} {i}.{ext}"),
                None => format!("{base_name} {i}"),
            };
            target = dir.join(nombre);
            i += 1;
        }

        match std::fs::rename(source, &target) {
            Ok(()) => {
                set_original_path(&target, source);
                return Ok(target);
            }
            Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
                // Volumenes distintos: la papelera del volumen no estaba
                // disponible y la de casa está en otro disco. Copiar y retirar.
                match copy_recursive(source, &target) {
                    Ok(()) => {
                        let _ = std::fs::remove_dir_all(source).or_else(|_| std::fs::remove_file(source));
                        set_original_path(&target, source);
                        return Ok(target);
                    }
                    Err(ce) => last_err = ce,
                }
            }
            Err(e) => last_err = format!("no se pudo mover a {}: {e}", dir.display()),
        }
    }
    Err(last_err)
}

/// Busca en las papeleras el elemento que hay que devolver a `original`.
///
/// Primero intenta el caso exacto —el atributo extendido guarda la ruta— y, si
/// nadie lo lleva (una papelera hecha desde el Finder, por ejemplo), cae al
/// nombre, eligiendo la coincidencia más reciente.
pub fn find_trashed_item(original: &Path) -> Option<PathBuf> {
    let target_name = original.file_name()?;
    let mut por_nombre: Vec<(u64, PathBuf)> = Vec::new();

    for dir in search_trash_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if get_original_path(&p).map(|o| o == original).unwrap_or(false) {
                return Some(p);
            }
            if entry.file_name() == target_name {
                let mtime = std::fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                por_nombre.push((mtime, p));
            }
        }
    }

    por_nombre.sort_by_key(|(m, _)| std::cmp::Reverse(*m));
    por_nombre.first().map(|(_, p)| p.clone())
}