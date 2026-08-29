use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use windows::core::PCWSTR;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    FindClose, FindFirstFileExW, FindNextFileW, FindExInfoBasic,
    FindExSearchNameMatch, FIND_FIRST_EX_LARGE_FETCH, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM, WIN32_FIND_DATAW,
};

#[derive(Debug, Clone)]
pub struct FsEntry {
    pub parent_path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub is_hidden: bool,
    pub is_system: bool,
    pub size: u64,
    pub mtime: u32,
    pub ctime: u32,
}

/// Convert Windows FILETIME (100-nanosecond intervals since Jan 1, 1601) to Unix timestamp
#[inline]
pub fn filetime_to_unix_secs(dw_low: u32, dw_high: u32) -> u32 {
    let ft = ((dw_high as u64) << 32) | (dw_low as u64);
    // Difference between 1601 and 1970 in 100-ns intervals: 116444736000000000
    const UNIX_EPOCH_DIFF: u64 = 116_444_736_000_000_000;
    if ft > UNIX_EPOCH_DIFF {
        ((ft - UNIX_EPOCH_DIFF) / 10_000_000) as u32
    } else {
        0
    }
}

/// High-speed non-recursive directory contents fetcher using FindFirstFileExW + Large Fetch
pub fn scan_directory(dir: &Path) -> Vec<FsEntry> {
    let mut entries = Vec::with_capacity(128);
    let mut search_pattern = dir.to_path_buf();
    search_pattern.push("*");

    let wide_pattern: Vec<u16> = search_pattern
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut find_data = WIN32_FIND_DATAW::default();

    let handle = unsafe {
        FindFirstFileExW(
            PCWSTR(wide_pattern.as_ptr()),
            FindExInfoBasic,
            &mut find_data as *mut _ as *mut _,
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };

    if let Ok(h) = handle
        && h != HANDLE(std::ptr::null_mut())
    {
        loop {
                let name_len = find_data
                    .cFileName
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(find_data.cFileName.len());

                let name = OsString::from_wide(&find_data.cFileName[..name_len])
                    .to_string_lossy()
                    .to_string();

                if name != "." && name != ".." {
                    let attrs = find_data.dwFileAttributes;
                    let is_dir = (attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0;
                    let is_hidden = (attrs & FILE_ATTRIBUTE_HIDDEN.0) != 0;
                    let is_system = (attrs & FILE_ATTRIBUTE_SYSTEM.0) != 0;

                    let size = if is_dir {
                        0
                    } else {
                        ((find_data.nFileSizeHigh as u64) << 32) | (find_data.nFileSizeLow as u64)
                    };

                    let mtime = filetime_to_unix_secs(
                        find_data.ftLastWriteTime.dwLowDateTime,
                        find_data.ftLastWriteTime.dwHighDateTime,
                    );
                    let ctime = filetime_to_unix_secs(
                        find_data.ftCreationTime.dwLowDateTime,
                        find_data.ftCreationTime.dwHighDateTime,
                    );

                    entries.push(FsEntry {
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

                let has_next = unsafe { FindNextFileW(h, &mut find_data) };
                if has_next.is_err() {
                    break;
                }
            }
            let _ = unsafe { FindClose(h) };
        }

    entries
}

/// Recursive scanner for exFAT / FAT32 volumes or folder subtrees
pub fn scan_subtree(root: &Path, max_depth: usize) -> Vec<FsEntry> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_current_dir() {
        let current_dir = std::env::current_dir().unwrap();
        let entries = scan_directory(&current_dir);
        assert!(!entries.is_empty(), "Current directory should contain entries");
        assert!(entries.iter().any(|e| e.name == "Cargo.toml" || e.name == "src"));
    }
}
