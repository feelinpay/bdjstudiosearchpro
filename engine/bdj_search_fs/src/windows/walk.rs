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
    pub is_reparse_point: bool,
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

                    let is_reparse_point = (attrs & 0x00000400) != 0;

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
                        is_reparse_point,
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

/// Recursive scanner for exFAT / FAT32 volumes or folder subtrees (BFS with user priority)
pub fn scan_subtree(root: &Path, max_depth: usize) -> Vec<FsEntry> {
    use std::collections::VecDeque;

    let mut all_entries = Vec::with_capacity(32768);
    let mut queue = VecDeque::new();
    queue.push_back((root.to_path_buf(), 0usize));

    while let Some((curr_dir, depth)) = queue.pop_front() {
        let entries = scan_directory(&curr_dir);
        for entry in entries {
            if entry.is_dir && !entry.is_reparse_point && depth < max_depth {
                // Avoid traversing junction loops, Recycle Bin, internal system volume metadata, and WinSxS
                if !entry.name.starts_with('$')
                    && !entry.name.eq_ignore_ascii_case("System Volume Information")
                    && !entry.name.eq_ignore_ascii_case("WinSxS")
                {
                    let mut sub_path = curr_dir.clone();
                    sub_path.push(&entry.name);

                    // Prioritize user directories (Downloads, Desktop, Music) to index user files first
                    if entry.name.eq_ignore_ascii_case("Users")
                        || entry.name.eq_ignore_ascii_case("Downloads")
                        || entry.name.eq_ignore_ascii_case("Desktop")
                        || entry.name.eq_ignore_ascii_case("Music")
                        || entry.name.eq_ignore_ascii_case("Videos")
                        || entry.name.eq_ignore_ascii_case("Documents")
                    {
                        queue.push_front((sub_path, depth + 1));
                    } else {
                        queue.push_back((sub_path, depth + 1));
                    }
                }
            }
            all_entries.push(entry);
        }
    }

    all_entries
}

/// Ultra-fast single-pass scanner that inserts entries directly into IndexBuilder.
/// Zero intermediate Vec allocations, BFS with user directory prioritization.
pub fn scan_subtree_into_builder(
    root: &Path,
    root_id: u32,
    vol_id: u8,
    builder: &mut bdj_search_core::index::builder::IndexBuilder,
) -> usize {
    use std::collections::VecDeque;

    let mut queue = VecDeque::with_capacity(4096);
    queue.push_back((root.to_path_buf(), root_id, 0usize));
    let mut count = 0usize;

    while let Some((curr_dir, parent_id, depth)) = queue.pop_front() {
        let mut search_pattern = curr_dir.clone();
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

        if let Ok(h) = handle && h != HANDLE(std::ptr::null_mut()) {
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
                    let is_reparse_point = (attrs & 0x00000400) != 0;

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

                    let entry_id = builder.add_entry(
                        parent_id,
                        &name,
                        is_dir,
                        is_hidden,
                        is_system,
                        vol_id,
                        size,
                        mtime,
                        ctime,
                    );
                    count += 1;

                    if is_dir
                        && !is_reparse_point
                        && !name.starts_with('$')
                        && !name.eq_ignore_ascii_case("System Volume Information")
                        && !name.eq_ignore_ascii_case("WinSxS")
                    {
                        let mut sub_path = curr_dir.clone();
                        sub_path.push(&name);

                        if name.eq_ignore_ascii_case("Users")
                            || name.eq_ignore_ascii_case("Downloads")
                            || name.eq_ignore_ascii_case("Desktop")
                            || name.eq_ignore_ascii_case("Music")
                            || name.eq_ignore_ascii_case("Videos")
                            || name.eq_ignore_ascii_case("Documents")
                        {
                            queue.push_front((sub_path, entry_id, depth + 1));
                        } else {
                            queue.push_back((sub_path, entry_id, depth + 1));
                        }
                    }
                }

                let has_next = unsafe { FindNextFileW(h, &mut find_data) };
                if has_next.is_err() {
                    break;
                }
            }
            let _ = unsafe { FindClose(h) };
        }
    }

    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_users_dir() {
        let temp_dir = std::env::temp_dir().join(format!("bdj_test_walk_{}", std::process::id()));
        let sub_dir = temp_dir.join("subfolder");
        let _ = std::fs::create_dir_all(&sub_dir);
        std::fs::write(temp_dir.join("file1.txt"), b"test content 1").unwrap();
        std::fs::write(sub_dir.join("file2.mp3"), b"test content 2").unwrap();

        let mut builder = bdj_search_core::index::builder::IndexBuilder::new();
        let vol_id = builder.vol_table.add_or_update(r"C:\", "Windows", "NTFS", false);
        let root_id = builder.add_entry(u32::MAX, "", true, false, false, vol_id, 0, 0, 0);

        let start = std::time::Instant::now();
        let count = scan_subtree_into_builder(&temp_dir, root_id, vol_id, &mut builder);
        println!("Indexed {} files in temp dir in {:?}", count, start.elapsed());
        assert!(count >= 2);

        let index_path = temp_dir.join("test_index.bdjx");
        let mut file = std::fs::File::create(&index_path).unwrap();
        let write_res = builder.write_to(&mut file);
        println!("Written index to {:?}: {:?} in {:?}", index_path, write_res, start.elapsed());
        assert!(write_res.is_ok());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
