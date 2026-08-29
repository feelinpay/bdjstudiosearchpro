use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct RawEntry {
    pub file_id: u64,
    pub parent_id: u64,
    pub name: String,
    pub is_dir: bool,
    pub is_hidden: bool,
    pub is_system: bool,
    pub size: u64,
    pub mtime: u32,
    pub ctime: u32,
}

#[derive(Debug, Clone)]
pub struct ScanReport {
    pub total_entries: u64,
    pub elapsed_ms: u64,
    pub errors: u32,
}

#[derive(Debug, Clone)]
pub enum Cursor {
    WindowsUsn { journal_id: u64, next_usn: i64 },
    MacOsEventId(u64),
    Timestamp(u64),
}

#[derive(Debug, Clone)]
pub enum FsChange {
    Created(RawEntry),
    Deleted { file_id: u64 },
    Renamed {
        file_id: u64,
        new_parent: u64,
        new_name: String,
    },
    Modified {
        file_id: u64,
        size: u64,
        mtime: u32,
    },
    RescanSubtree { path: PathBuf },
}

pub trait VolumeScanner: Send {
    fn scan(&mut self, sink: &mut dyn FnMut(&[RawEntry])) -> Result<ScanReport, Box<dyn std::error::Error + Send + Sync>>;
}

pub trait ChangeWatcher: Send {
    fn poll(&mut self, timeout: Duration) -> Result<Vec<FsChange>, Box<dyn std::error::Error + Send + Sync>>;
    fn cursor(&self) -> Cursor;
    fn resume_from(&mut self, cursor: Cursor) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}
