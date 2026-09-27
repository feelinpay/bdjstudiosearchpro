use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone)]
pub struct MacFsEntry {
    pub parent_path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub is_hidden: bool,
    pub is_system: bool,
    pub size: u64,
    pub mtime: u32,
    pub ctime: u32,
}

/// Checks if directory is a standard macOS system folder to be excluded
pub fn is_excluded_macos_dir(name: &str) -> bool {
    matches!(
        name,
        ".Spotlight-V100"
            | ".Trashes"
            | ".fseventsd"
            | ".DocumentRevisions-V100"
            | ".MobileBackups"
            | ".TemporaryItems"
            | ".com.apple.TimeMachine.supported"
    )
}

#[cfg(target_os = "macos")]
mod ffi {
    use libc::{c_int, c_void, size_t};

    #[repr(C)]
    #[derive(Debug, Default, Copy, Clone)]
    pub struct AttrList {
        pub bitmapcount: u16,
        pub reserved: u16,
        pub commonattr: u32,
        pub volattr: u32,
        pub dirattr: u32,
        pub fileattr: u32,
        pub forkattr: u32,
    }

    pub const ATTR_BIT_MAP_COUNT: u16 = 5;
    pub const ATTR_CMN_NAME: u32 = 0x00000001;
    pub const ATTR_CMN_OBJTYPE: u32 = 0x00000008;
    pub const ATTR_CMN_CRTIME: u32 = 0x00000200;
    pub const ATTR_CMN_MODTIME: u32 = 0x00000400;
    pub const ATTR_CMN_RETURNED_ATTRS: u32 = 0x80000000;
    pub const ATTR_FILE_DATALENGTH: u32 = 0x00000200;

    pub const VDIR: u32 = 2;

    unsafe extern "C" {
        pub fn getattrlistbulk(
            dirfd: c_int,
            attrList: *mut AttrList,
            attrBuf: *mut c_void,
            attrBufSize: size_t,
            options: u64,
        ) -> c_int;
    }
}

/// Escanea un directorio mediante `getattrlistbulk(2)`, devolviendo lotes
/// de entradas con sus metadatos en una sola llamada al sistema sin `lstat`.
#[cfg(target_os = "macos")]
fn scan_directory_bulk(dir: &Path) -> Option<Vec<MacFsEntry>> {
    use std::ffi::{CStr, CString};
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(dir.as_os_str().as_bytes()).ok()?;
    let dirfd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY) };
    if dirfd < 0 {
        return None;
    }

    struct DirCloser(libc::c_int);
    impl Drop for DirCloser {
        fn drop(&mut self) {
            unsafe { libc::close(self.0) };
        }
    }
    let _closer = DirCloser(dirfd);

    let mut attr_list = ffi::AttrList {
        bitmapcount: ffi::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: ffi::ATTR_CMN_RETURNED_ATTRS
            | ffi::ATTR_CMN_NAME
            | ffi::ATTR_CMN_OBJTYPE
            | ffi::ATTR_CMN_CRTIME
            | ffi::ATTR_CMN_MODTIME,
        volattr: 0,
        dirattr: 0,
        fileattr: ffi::ATTR_FILE_DATALENGTH,
        forkattr: 0,
    };

    // Buffer de 128 KB para recopilar cientos de entradas por syscall
    let mut buf = vec![0u8; 128 * 1024];
    let mut entries = Vec::with_capacity(128);

    loop {
        let count = unsafe {
            ffi::getattrlistbulk(
                dirfd,
                &mut attr_list,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                0,
            )
        };

        if count < 0 {
            // Si el sistema de archivos no soporta getattrlistbulk, retornar None para fallback a readdir
            return if entries.is_empty() { None } else { Some(entries) };
        }
        if count == 0 {
            break;
        }

        let mut offset = 0usize;
        for _ in 0..count {
            if offset + 4 > buf.len() {
                break;
            }
            let entry_len = unsafe {
                std::ptr::read_unaligned(buf.as_ptr().add(offset) as *const u32) as usize
            };
            if entry_len == 0 || offset + entry_len > buf.len() {
                break;
            }

            let entry_ptr = unsafe { buf.as_ptr().add(offset) };
            // El kernel de macOS empaqueta los atributos a 4 bytes:
            // offset 0:  entry_len (u32, 4 bytes)
            // offset 4:  returned_attrs (5 * u32 = 20 bytes: común, vol, dir, file, fork)
            // offset 24: AttrReference para nombre (attr_dataoffset i32, attr_length u32 = 8 bytes)
            // offset 32: ATTR_CMN_OBJTYPE (u32 = 4 bytes)
            // offset 36: ATTR_CMN_CRTIME (timespec tv_sec: i64, tv_nsec: i64 = 16 bytes)
            // offset 52: ATTR_CMN_MODTIME (timespec tv_sec: i64, tv_nsec: i64 = 16 bytes)
            // offset 68: ATTR_FILE_DATALENGTH (u64 = 8 bytes)

            let ret_file = unsafe {
                std::ptr::read_unaligned(entry_ptr.add(16) as *const u32)
            };

            // AttrReference en offset 24
            let rel_offset = unsafe {
                std::ptr::read_unaligned(entry_ptr.add(24) as *const i32)
            };
            let name_ptr = unsafe { entry_ptr.add(24).offset(rel_offset as isize) };
            let name = unsafe { CStr::from_ptr(name_ptr as *const libc::c_char) }
                .to_string_lossy()
                .to_string();

            if !is_excluded_macos_dir(&name) && name != "." && name != ".." {
                // obj_type en offset 32
                let obj_type = unsafe {
                    std::ptr::read_unaligned(entry_ptr.add(32) as *const u32)
                };
                let is_dir = obj_type == ffi::VDIR;

                // timespec tv_sec empaquetado a 4 bytes en offset 36 (ctime) y 52 (mtime)
                let ctime_sec = unsafe {
                    std::ptr::read_unaligned(entry_ptr.add(36) as *const i64)
                };
                let mtime_sec = unsafe {
                    std::ptr::read_unaligned(entry_ptr.add(52) as *const i64)
                };

                let ctime = ctime_sec.max(0) as u32;
                let mtime = mtime_sec.max(0) as u32;

                // Tamaño en offset 68
                let size = if is_dir {
                    0
                } else if (ret_file & ffi::ATTR_FILE_DATALENGTH) != 0 {
                    unsafe { std::ptr::read_unaligned(entry_ptr.add(68) as *const u64) }
                } else {
                    0
                };

                let is_hidden = name.starts_with('.');
                let is_system = name.starts_with('.') || is_excluded_macos_dir(&name);

                entries.push(MacFsEntry {
                    parent_path: dir.to_path_buf(),
                    name,
                    is_dir,
                    is_hidden,
                    is_system,
                    size,
                    mtime,
                    ctime,
                });
            }

            offset += entry_len;
        }
    }

    Some(entries)
}

/// Fallback portable basado en `fs::read_dir` para sistemas o volúmenes
/// donde `getattrlistbulk` no está disponible.
fn scan_directory_fallback(dir: &Path) -> Vec<MacFsEntry> {
    let mut entries = Vec::with_capacity(128);

    if let Ok(read_dir) = fs::read_dir(dir) {
        for entry in read_dir.flatten() {
            let name = match entry.file_name().into_string() {
                Ok(n) => n,
                Err(os_str) => os_str.to_string_lossy().to_string(),
            };

            if is_excluded_macos_dir(&name) {
                continue;
            }

            let is_hidden = name.starts_with('.');
            let is_system = name.starts_with('.') || is_excluded_macos_dir(&name);

            if let Ok(metadata) = entry.metadata() {
                let is_dir = metadata.is_dir();
                let size = if is_dir { 0 } else { metadata.len() };

                let mtime = metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as u32)
                    .unwrap_or(0);

                let ctime = metadata
                    .created()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as u32)
                    .unwrap_or(mtime);

                entries.push(MacFsEntry {
                    parent_path: dir.to_path_buf(),
                    name,
                    is_dir,
                    is_hidden,
                    is_system,
                    size,
                    mtime,
                    ctime,
                });
            }
        }
    }

    entries
}

/// Scans single directory on macOS extracting metadata
pub fn scan_directory(dir: &Path) -> Vec<MacFsEntry> {
    #[cfg(target_os = "macos")]
    {
        if let Some(entries) = scan_directory_bulk(dir) {
            return entries;
        }
    }

    scan_directory_fallback(dir)
}

/// Recursive subtree scanner for APFS/HFS+ with max depth bounding
pub fn scan_subtree(root: &Path, max_depth: usize) -> Vec<MacFsEntry> {
    let mut all_entries = Vec::with_capacity(1024);
    let mut queue = vec![(root.to_path_buf(), 0usize)];

    while let Some((curr_dir, depth)) = queue.pop() {
        let entries = scan_directory(&curr_dir);
        for entry in entries {
            if entry.is_dir && depth < max_depth {
                let mut sub_path = curr_dir.clone();
                sub_path.push(&entry.name);
                queue.push((sub_path, depth + 1));
            }
            all_entries.push(entry);
        }
    }

    all_entries
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_getattrlistbulk_coincide_con_fallback_y_metadata() {
        let temp = tempdir().expect("crear directorio temporal");
        let root = temp.path();

        // 1. Archivo regular con tamaño conocido (ej. 1234 bytes)
        let file1_path = root.join("archivo1.txt");
        let mut f1 = File::create(&file1_path).unwrap();
        f1.write_all(&vec![b'A'; 1234]).unwrap();
        drop(f1);

        // Fijar mtime a 2035-01-01 00:00:00 UTC (2_051_222_400) mediante libc::utimes.
        //
        // La fecha va en el FUTURO a propósito. En macOS, `utimes` con una
        // fecha de modificación ANTERIOR a la de creación arrastra también la
        // fecha de creación hacia atrás (birthtime = mtime), y entonces crtime
        // y mtime salen idénticos: la prueba no podría distinguir si los
        // desplazamientos 36 (crtime) y 52 (modtime) están intercambiados.
        // Con una fecha futura la creación se queda en «ahora».
        let target_mtime: libc::time_t = 2_051_222_400;
        let times = [
            libc::timeval {
                tv_sec: target_mtime,
                tv_usec: 0,
            },
            libc::timeval {
                tv_sec: target_mtime,
                tv_usec: 0,
            },
        ];
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let c_path = CString::new(file1_path.as_os_str().as_bytes()).unwrap();
        let ut_res = unsafe { libc::utimes(c_path.as_ptr(), times.as_ptr()) };
        assert_eq!(ut_res, 0, "libc::utimes debe completarse con éxito");

        // 2. Archivo vacío (0 bytes)
        let file2_path = root.join("vacio.bin");
        File::create(&file2_path).unwrap();

        // 3. Archivo oculto (comienza con .)
        let hidden_path = root.join(".config_oculto");
        let mut f_h = File::create(&hidden_path).unwrap();
        f_h.write_all(b"secreto").unwrap();
        drop(f_h);

        // 4. Subcarpeta
        let subdir_path = root.join("mi_subcarpeta");
        fs::create_dir(&subdir_path).unwrap();

        // 5. Archivo con nombre largo y caracteres no ASCII para verificar desplazamiento del nombre
        let unicode_name = "Canción — remix ñandú (extended).wav";
        let unicode_path = root.join(unicode_name);
        let mut f_u = File::create(&unicode_path).unwrap();
        f_u.write_all(b"audio data").unwrap();
        drop(f_u);

        // Ejecutar ambos métodos
        let bulk = scan_directory_bulk(root).expect("scan_directory_bulk debe funcionar en macOS");
        let fallback = scan_directory_fallback(root);

        assert_eq!(bulk.len(), 5, "debe encontrar exactamente 5 entradas");
        assert_eq!(fallback.len(), 5, "fallback debe encontrar exactamente 5 entradas");

        let mut bulk_sorted = bulk.clone();
        bulk_sorted.sort_by(|a, b| a.name.cmp(&b.name));

        let mut fallback_sorted = fallback.clone();
        fallback_sorted.sort_by(|a, b| a.name.cmp(&b.name));

        for (b, fb) in bulk_sorted.iter().zip(fallback_sorted.iter()) {
            assert_eq!(b.name, fb.name, "los nombres deben coincidir");
            assert_eq!(b.is_dir, fb.is_dir, "is_dir debe coincidir para {}", b.name);
            assert_eq!(b.is_hidden, fb.is_hidden, "is_hidden debe coincidir para {}", b.name);
            assert_eq!(b.size, fb.size, "el tamaño debe coincidir para {}", b.name);

            // Las fechas mtime y ctime deben coincidir dentro de 1 segundo de resolución
            assert!(
                (b.mtime as i64 - fb.mtime as i64).abs() <= 1,
                "mtime debe coincidir para {}: bulk={}, fb={}",
                b.name, b.mtime, fb.mtime
            );
            assert!(
                (b.ctime as i64 - fb.ctime as i64).abs() <= 1,
                "ctime debe coincidir para {}: bulk={}, fb={}",
                b.name, b.ctime, fb.ctime
            );

            // archivo1.txt: mtime exacto en 2035 y crtime independiente («ahora»).
            if b.name == "archivo1.txt" {
                assert_eq!(b.mtime, 2_051_222_400, "mtime debe ser exactamente el fijado en 2035");
                assert_ne!(b.ctime, b.mtime, "ctime (creación) debe ser distinto de mtime (2035)");
            }

            // Verificar también contra std::fs::metadata
            let item_path = root.join(&b.name);
            let meta = fs::metadata(&item_path).unwrap();
            assert_eq!(b.is_dir, meta.is_dir());

            // Comprobación independiente del fallback: cada fecha contra el
            // campo que le corresponde en `metadata`. Si 36 y 52 estuvieran
            // intercambiados, esto falla aunque bulk y fallback coincidieran.
            let secs = |t: std::io::Result<std::time::SystemTime>| {
                t.ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
            };
            if let Some(m) = secs(meta.modified()) {
                assert!((b.mtime as i64 - m).abs() <= 1, "mtime vs metadata para {}: {} != {}", b.name, b.mtime, m);
            }
            if let Some(c) = secs(meta.created()) {
                assert!((b.ctime as i64 - c).abs() <= 1, "crtime vs metadata para {}: {} != {}", b.name, b.ctime, c);
            }
            if !b.is_dir {
                assert_eq!(b.size, meta.len());
            }
        }
    }
}
