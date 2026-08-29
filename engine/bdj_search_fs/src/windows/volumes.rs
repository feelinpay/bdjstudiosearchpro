#[derive(Debug, Clone)]
pub struct VolumeInfo {
    pub id: u8,
    pub path: String,
    pub label: String,
    pub fs_type: String,
    pub is_removable: bool,
    pub is_ready: bool,
}

pub fn list_volumes() -> Vec<VolumeInfo> {
    Vec::new()
}
