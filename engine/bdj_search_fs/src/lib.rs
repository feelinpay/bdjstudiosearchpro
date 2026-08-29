pub mod traits;

#[cfg(windows)]
pub mod windows;

#[cfg(target_os = "macos")]
pub mod macos;

pub use traits::{ChangeWatcher, Cursor, FsChange, RawEntry, ScanReport, VolumeScanner};
