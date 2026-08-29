// Directory walk fallback for exFAT/FAT32/Network
use crate::traits::{RawEntry, ScanReport, VolumeScanner};
use std::path::PathBuf;

pub struct DirectoryWalkScanner {
    pub root: PathBuf,
}

impl DirectoryWalkScanner {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl VolumeScanner for DirectoryWalkScanner {
    fn scan(&mut self, _sink: &mut dyn FnMut(&[RawEntry])) -> Result<ScanReport, Box<dyn std::error::Error + Send + Sync>> {
        Ok(ScanReport {
            total_entries: 0,
            elapsed_ms: 0,
            errors: 0,
        })
    }
}
