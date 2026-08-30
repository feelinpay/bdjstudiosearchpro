use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadDirectoryChangesW,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_ATTRIBUTES,
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
    FILE_NOTIFY_CHANGE_SIZE, FILE_NOTIFY_INFORMATION, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};

#[derive(Debug, Clone)]
pub struct DirectoryChangeRecord {
    pub action: u32,
    pub relative_path: String,
    pub is_dir: bool,
}

pub struct DirectoryWatcher {
    handle: HANDLE,
    pub root_path: PathBuf,
}

impl Drop for DirectoryWatcher {
    fn drop(&mut self) {
        if self.handle != INVALID_HANDLE_VALUE && self.handle != HANDLE(std::ptr::null_mut()) {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

impl DirectoryWatcher {
    /// Opens directory for synchronous or asynchronous ReadDirectoryChangesW notification.
    pub fn open(path: &Path) -> Result<Self, windows::core::Error> {
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_LIST_DIRECTORY.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                None,
            )?
        };

        Ok(Self {
            handle,
            root_path: path.to_path_buf(),
        })
    }

    /// Reads changes synchronously into a buffer and executes callback for each record.
    pub fn read_changes<F>(&self, buffer: &mut [u8], mut callback: F) -> Result<usize, windows::core::Error>
    where
        F: FnMut(DirectoryChangeRecord),
    {
        let filter = FILE_NOTIFY_CHANGE_FILE_NAME
            | FILE_NOTIFY_CHANGE_DIR_NAME
            | FILE_NOTIFY_CHANGE_ATTRIBUTES
            | FILE_NOTIFY_CHANGE_SIZE
            | FILE_NOTIFY_CHANGE_LAST_WRITE;

        let mut bytes_returned = 0u32;
        unsafe {
            ReadDirectoryChangesW(
                self.handle,
                buffer.as_mut_ptr() as *mut _,
                buffer.len() as u32,
                true, // recursive subtree
                filter,
                Some(&mut bytes_returned),
                None,
                None,
            )?;
        }

        if bytes_returned == 0 {
            return Ok(0);
        }

        let mut count = 0;
        let mut offset = 0usize;

        while offset < bytes_returned as usize {
            let info_ptr = buffer[offset..].as_ptr() as *const FILE_NOTIFY_INFORMATION;
            let info = unsafe { &*info_ptr };

            let name_len = (info.FileNameLength as usize) / 2;
            let name_slice = unsafe {
                std::slice::from_raw_parts(
                    buffer[offset + std::mem::size_of::<FILE_NOTIFY_INFORMATION>() - 4..].as_ptr()
                        as *const u16,
                    name_len,
                )
            };
            let relative_path = OsString::from_wide(name_slice)
                .to_string_lossy()
                .to_string();

            let full_target = self.root_path.join(&relative_path);
            let is_dir = full_target.is_dir();

            callback(DirectoryChangeRecord {
                action: info.Action.0,
                relative_path,
                is_dir,
            });
            count += 1;

            if info.NextEntryOffset == 0 {
                break;
            }
            offset += info.NextEntryOffset as usize;
        }

        Ok(count)
    }
}
