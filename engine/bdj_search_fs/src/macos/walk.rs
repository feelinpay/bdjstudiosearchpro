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
