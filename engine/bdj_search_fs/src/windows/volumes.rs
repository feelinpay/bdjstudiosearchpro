use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{
    GetDriveTypeW, GetLogicalDriveStringsW, GetVolumeInformationW,
};

const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOTE: u32 = 4;
const DRIVE_CDROM: u32 = 5;
const DRIVE_RAMDISK: u32 = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeInfo {
    pub id: u8,
    pub path: String,       // e.g. "C:\"
    pub label: String,      // e.g. "Windows"
    pub fs_type: String,    // e.g. "NTFS", "exFAT", "FAT32"
    pub is_removable: bool,
    pub is_ready: bool,
    pub is_ntfs: bool,
}

pub fn list_volumes() -> Vec<VolumeInfo> {
    let mut volumes = Vec::new();

    // 1. Query logical drive strings
    let mut buffer = [0u16; 512];
    let len = unsafe { GetLogicalDriveStringsW(Some(&mut buffer)) };
    if len == 0 || len as usize > buffer.len() {
        return volumes;
    }

    // Split drives at null characters
    let mut start = 0;
    let mut vol_id = 0u8;

    for i in 0..len as usize {
        if buffer[i] == 0 {
            if i > start {
                let drive_slice = &buffer[start..i];
                let drive_path = OsString::from_wide(drive_slice).to_string_lossy().to_string();

                let mut wide_drive: Vec<u16> = drive_slice.to_vec();
                wide_drive.push(0);

                let drive_type = unsafe { GetDriveTypeW(PCWSTR(wide_drive.as_ptr())) };

                // Skip CD-ROM drives or unknown types that are inaccessible
                if drive_type == DRIVE_CDROM {
                    start = i + 1;
                    continue;
                }

                let is_removable = drive_type == DRIVE_REMOVABLE;
                let is_remote = drive_type == DRIVE_REMOTE;
                let _is_fixed = drive_type == DRIVE_FIXED || drive_type == DRIVE_RAMDISK;

                // Query Volume Information (Label and FS Type)
                let mut vol_name = [0u16; 256];
                let mut fs_name = [0u16; 256];
                let mut serial = 0u32;
                let mut max_comp_len = 0u32;
                let mut flags = 0u32;

                let success = unsafe {
                    GetVolumeInformationW(
                        PCWSTR(wide_drive.as_ptr()),
                        Some(&mut vol_name),
                        Some(&mut serial),
                        Some(&mut max_comp_len),
                        Some(&mut flags),
                        Some(&mut fs_name),
                    )
                }
                .is_ok();

                let (label, fs_type, is_ready) = if success {
                    let name_len = vol_name.iter().position(|&c| c == 0).unwrap_or(vol_name.len());
                    let fs_len = fs_name.iter().position(|&c| c == 0).unwrap_or(fs_name.len());

                    let l = OsString::from_wide(&vol_name[..name_len]).to_string_lossy().to_string();
                    let f = OsString::from_wide(&fs_name[..fs_len]).to_string_lossy().to_string();
                    (l, f, true)
                } else {
                    (String::new(), if is_remote { "Network".to_string() } else { "Unknown".to_string() }, false)
                };

                let is_ntfs = fs_type.eq_ignore_ascii_case("NTFS") || fs_type.eq_ignore_ascii_case("ReFS");

                volumes.push(VolumeInfo {
                    id: vol_id,
                    path: drive_path,
                    label,
                    fs_type,
                    is_removable,
                    is_ready,
                    is_ntfs,
                });

                vol_id = vol_id.saturating_add(1);
            }
            start = i + 1;
        }
    }

    volumes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_volumes_windows() {
        let vols = list_volumes();
        println!("Detected Windows volumes: {:?}", vols);
        // At least system drive C:\ must be detected on Windows
        assert!(!vols.is_empty());
        assert!(vols.iter().any(|v| v.path.starts_with("C:")));
    }
}
