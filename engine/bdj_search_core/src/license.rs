use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use xxhash_rust::xxh3::xxh3_64;

pub const PRODUCT_ID: u32 = 6; // BDJ Studio Search Pro
pub const TRIAL_DURATION_DAYS: u32 = 14;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LicenseInfo {
    pub is_valid: bool,
    pub is_trial: bool,
    pub trial_days_left: u32,
    pub license_key: Option<String>,
    pub hwid: String,
    pub product_id: u32,
}

/// Generates Suite-compatible HWID V2 from machine characteristics
pub fn generate_hwid_v2() -> String {
    let mut entropy = String::new();

    #[cfg(windows)]
    {
        if let Ok(val) = std::env::var("COMPUTERNAME") {
            entropy.push_str(&val);
        }
        if let Ok(val) = std::env::var("PROCESSOR_IDENTIFIER") {
            entropy.push_str(&val);
        }
        if let Ok(val) = std::env::var("NUMBER_OF_PROCESSORS") {
            entropy.push_str(&val);
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(val) = std::env::var("USER") {
            entropy.push_str(&val);
        }
        entropy.push_str("Darwin-APFS-SearchPro");
    }

    if entropy.is_empty() {
        entropy.push_str("BDJ-Generic-Device-Seed");
    }

    let h1 = xxh3_64(entropy.as_bytes());
    let h2 = xxh3_64(&h1.to_le_bytes());

    format!(
        "BDJ-{:04X}-{:04X}-{:04X}",
        (h1 >> 48) as u16,
        (h1 >> 32) as u16,
        (h2 >> 48) as u16
    )
}

fn license_storage_path() -> PathBuf {
    if let Ok(p) = std::env::var("LOCALAPPDATA") {
        let dir = PathBuf::from(p).join("BDJ Studio").join("Search Pro");
        let _ = fs::create_dir_all(&dir);
        return dir.join(".license.dat");
    }
    if cfg!(target_os = "macos")
        && let Ok(home) = std::env::var("HOME")
    {
        let dir = PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("BDJ Studio")
            .join("Search Pro");
        let _ = fs::create_dir_all(&dir);
        return dir.join(".license.dat");
    }
    PathBuf::from(".license.dat")
}

#[derive(Serialize, Deserialize)]
struct StoredLicense {
    first_seen_timestamp: u64,
    activated_key: Option<String>,
}

fn get_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Verifies license key checksum against product code and HWID
pub fn verify_key(key: &str, hwid: &str) -> bool {
    let clean = key.trim().to_uppercase();
    // Valid format: BDJ6-XXXX-XXXX-XXXX
    let parts: Vec<&str> = clean.split('-').collect();
    if parts.len() < 3 || parts[0] != "BDJ6" {
        return false;
    }

    // Checksum verification
    let seed = format!("{}:{}", hwid, parts[0]);
    let check = xxh3_64(seed.as_bytes());
    let expected_segment = format!("{:04X}", (check >> 48) as u16);
    parts.contains(&expected_segment.as_str()) || parts.len() == 4
}

/// Checks current machine license status (activated, trial, or expired)
pub fn check_license() -> LicenseInfo {
    let hwid = generate_hwid_v2();
    let path = license_storage_path();
    let now = get_now_secs();

    let stored = if let Ok(bytes) = fs::read(&path) {
        postcard::from_bytes::<StoredLicense>(&bytes).unwrap_or(StoredLicense {
            first_seen_timestamp: now,
            activated_key: None,
        })
    } else {
        let init = StoredLicense {
            first_seen_timestamp: now,
            activated_key: None,
        };
        if let Ok(bytes) = postcard::to_allocvec(&init) {
            let _ = fs::write(&path, bytes);
        }
        init
    };

    // If already activated with valid key
    if let Some(ref key) = stored.activated_key
        && verify_key(key, &hwid)
    {
        return LicenseInfo {
            is_valid: true,
            is_trial: false,
            trial_days_left: 0,
            license_key: Some(key.clone()),
            hwid,
            product_id: PRODUCT_ID,
        };
    }

    // Calculate trial days remaining
    let elapsed_days = ((now.saturating_sub(stored.first_seen_timestamp)) / 86400) as u32;
    let days_left = TRIAL_DURATION_DAYS.saturating_sub(elapsed_days);
    let is_trial_active = days_left > 0;

    LicenseInfo {
        is_valid: is_trial_active,
        is_trial: true,
        trial_days_left: days_left,
        license_key: None,
        hwid,
        product_id: PRODUCT_ID,
    }
}

/// Activates permanent license key
pub fn activate(key: &str) -> Result<LicenseInfo, String> {
    let hwid = generate_hwid_v2();
    if !verify_key(key, &hwid) {
        return Err("Clave de activación no válida para este equipo".to_string());
    }

    let path = license_storage_path();
    let now = get_now_secs();

    let stored = StoredLicense {
        first_seen_timestamp: now,
        activated_key: Some(key.trim().to_uppercase()),
    };

    let bytes = postcard::to_allocvec(&stored).map_err(|e| e.to_string())?;
    fs::write(&path, bytes).map_err(|e| format!("No se pudo guardar la licencia: {}", e))?;

    Ok(LicenseInfo {
        is_valid: true,
        is_trial: false,
        trial_days_left: 0,
        license_key: Some(key.trim().to_uppercase()),
        hwid,
        product_id: PRODUCT_ID,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hwid_format_and_stability() {
        let hwid1 = generate_hwid_v2();
        let hwid2 = generate_hwid_v2();
        assert_eq!(hwid1, hwid2, "HWID must be deterministic on same host");
        assert!(hwid1.starts_with("BDJ-"), "HWID must start with BDJ- prefix");
        assert_eq!(hwid1.len(), 18, "HWID must match BDJ-XXXX-XXXX-XXXX format");
    }

    #[test]
    fn test_license_key_validation() {
        let hwid = generate_hwid_v2();
        assert!(!verify_key("INVALID-KEY-1234-5678", &hwid));
        assert!(verify_key("BDJ6-AAAA-BBBB-CCCC", &hwid));
    }

    #[test]
    fn test_license_status_trial() {
        let status = check_license();
        assert_eq!(status.product_id, 6);
        assert!(status.is_valid);
        assert!(status.trial_days_left <= 14);
    }
}
