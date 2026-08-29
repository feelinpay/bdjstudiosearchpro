use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacVolumeInfo {
    pub id: u8,
    pub path: PathBuf,
    pub label: String,
    pub fs_type: String,
    pub is_removable: bool,
    pub is_ready: bool,
}

pub fn list_volumes() -> Vec<MacVolumeInfo> {
    let mut volumes = Vec::new();
    let mut id = 0u8;

    // 1. Primary System Volume (APFS)
    let root_path = PathBuf::from("/");
    if root_path.exists() {
        volumes.push(MacVolumeInfo {
            id,
            path: root_path,
            label: "Macintosh HD".to_string(),
            fs_type: "APFS".to_string(),
            is_removable: false,
            is_ready: true,
        });
        id += 1;
    }

    // 2. Data Volume (/System/Volumes/Data)
    let data_path = PathBuf::from("/System/Volumes/Data");
    if data_path.exists() {
        volumes.push(MacVolumeInfo {
            id,
            path: data_path,
            label: "Data".to_string(),
            fs_type: "APFS".to_string(),
            is_removable: false,
            is_ready: true,
        });
        id += 1;
    }

    // 3. External & Removable Volumes (/Volumes/*)
    let ext_volumes_dir = Path::new("/Volumes");
    if let Ok(entries) = std::fs::read_dir(ext_volumes_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_symlink() || path == Path::new("/Volumes/Macintosh HD") {
                continue;
            }
            if path.is_dir() {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| format!("Volume {}", id));

                volumes.push(MacVolumeInfo {
                    id,
                    path,
                    label: name,
                    fs_type: "External".to_string(),
                    is_removable: true,
                    is_ready: true,
                });
                id = id.saturating_add(1);
            }
        }
    }

    volumes
}
