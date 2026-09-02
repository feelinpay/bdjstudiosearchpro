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

/// Scans single directory on macOS extracting metadata
pub fn scan_directory(dir: &Path) -> Vec<MacFsEntry> {
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
