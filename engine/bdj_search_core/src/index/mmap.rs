use super::view::{IndexView, ViewError};
use memmap2::Mmap;
use std::fs::File;
use std::io;
use std::path::Path;

pub struct MmapIndex {
    mmap: Mmap,
}

impl MmapIndex {
    /// Opens the .bdjx file in read-only memory-mapped mode.
    /// Uses FILE_SHARE_DELETE on Windows to ensure atomic compaction
    /// can rename/replace the file without sharing violations.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let file = Self::open_file_shared(path.as_ref())?;
        // Safety: The mapped file is opened read-only, and Section 5.3 of the Master Plan
        // guarantees immutable base access with generation-based atomicity.
        let mmap = unsafe { Mmap::map(&file)? };
        Ok(Self { mmap })
    }

    #[cfg(windows)]
    fn open_file_shared(path: &Path) -> io::Result<File> {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 0x00000001;
        const FILE_SHARE_WRITE: u32 = 0x00000002;
        const FILE_SHARE_DELETE: u32 = 0x00000004;

        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .open(path)
    }

    #[cfg(not(windows))]
    fn open_file_shared(path: &Path) -> io::Result<File> {
        File::open(path)
    }

    pub fn view(&self) -> Result<IndexView<'_>, ViewError> {
        IndexView::from_bytes(&self.mmap)
    }

    pub fn len(&self) -> usize {
        self.mmap.len()
    }

    pub fn is_empty(&self) -> bool {
        self.mmap.is_empty()
    }
}
