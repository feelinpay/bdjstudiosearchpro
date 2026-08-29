// USN Journal and FSCTL_ENUM_USN_DATA implementation
use crate::traits::{RawEntry, ScanReport, VolumeScanner};

pub struct UsnScanner {
    pub volume_letter: char,
}

impl UsnScanner {
    pub fn new(volume_letter: char) -> Self {
        Self { volume_letter }
    }
}

impl VolumeScanner for UsnScanner {
    fn scan(&mut self, _sink: &mut dyn FnMut(&[RawEntry])) -> Result<ScanReport, Box<dyn std::error::Error + Send + Sync>> {
        Ok(ScanReport {
            total_entries: 0,
            elapsed_ms: 0,
            errors: 0,
        })
    }
}
