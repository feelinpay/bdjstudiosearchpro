use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::{
    FSCTL_ENUM_USN_DATA, FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_USN_JOURNAL,
    MFT_ENUM_DATA_V0, READ_USN_JOURNAL_DATA_V0, USN_JOURNAL_DATA_V0, USN_RECORD_V2,
};
use windows::Win32::System::IO::DeviceIoControl;

#[derive(Debug, Clone)]
pub struct UsnRecord {
    pub file_ref: u64,
    pub parent_file_ref: u64,
    pub usn: i64,
    pub reason: u32,
    pub attributes: u32,
    pub name: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct UsnJournalCursor {
    pub journal_id: u64,
    pub next_usn: i64,
}

pub struct UsnScanner {
    handle: HANDLE,
    pub drive_letter: char,
}

impl Drop for UsnScanner {
    fn drop(&mut self) {
        if self.handle != INVALID_HANDLE_VALUE && self.handle != HANDLE(std::ptr::null_mut()) {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

impl UsnScanner {
    /// Opens the raw NTFS volume (e.g. "\\.\C:") with elevated read privileges.
    pub fn open(drive_letter: char) -> Result<Self, windows::core::Error> {
        let path = format!("\\\\.\\{}:", drive_letter);
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )?
        };

        Ok(Self {
            handle,
            drive_letter,
        })
    }

    /// Queries journal state: journal_id and next_usn cursor
    pub fn query_journal(&self) -> Result<UsnJournalCursor, windows::core::Error> {
        let mut journal_data = USN_JOURNAL_DATA_V0::default();
        let mut bytes_returned = 0u32;

        unsafe {
            DeviceIoControl(
                self.handle,
                FSCTL_QUERY_USN_JOURNAL,
                None,
                0,
                Some(&mut journal_data as *mut _ as *mut _),
                std::mem::size_of::<USN_JOURNAL_DATA_V0>() as u32,
                Some(&mut bytes_returned),
                None,
            )?;
        }

        Ok(UsnJournalCursor {
            journal_id: journal_data.UsnJournalID,
            next_usn: journal_data.NextUsn,
        })
    }

    /// Enumerate all existing MFT records on the NTFS volume using FSCTL_ENUM_USN_DATA
    pub fn enumerate_all<F>(&self, mut callback: F) -> Result<UsnJournalCursor, windows::core::Error>
    where
        F: FnMut(UsnRecord),
    {
        let cursor = self.query_journal()?;
        let mut enum_data = MFT_ENUM_DATA_V0 {
            StartFileReferenceNumber: 0,
            LowUsn: 0,
            HighUsn: cursor.next_usn,
        };

        // 64 KB buffer for efficient MFT streaming
        let mut buffer = vec![0u8; 64 * 1024];

        loop {
            let mut bytes_returned = 0u32;
            let success = unsafe {
                DeviceIoControl(
                    self.handle,
                    FSCTL_ENUM_USN_DATA,
                    Some(&enum_data as *const _ as *const _),
                    std::mem::size_of::<MFT_ENUM_DATA_V0>() as u32,
                    Some(buffer.as_mut_ptr() as *mut _),
                    buffer.len() as u32,
                    Some(&mut bytes_returned),
                    None,
                )
            };

            if success.is_err() || bytes_returned < 8 {
                break;
            }

            // Next starting FRN is stored in the first 8 bytes of the output buffer
            let next_frn = u64::from_le_bytes(buffer[0..8].try_into().unwrap());
            enum_data.StartFileReferenceNumber = next_frn;

            // Parse USN_RECORD entries in buffer starting at byte 8
            let mut offset = 8usize;
            while offset + std::mem::size_of::<USN_RECORD_V2>() <= bytes_returned as usize {
                let rec_ptr = buffer[offset..].as_ptr() as *const USN_RECORD_V2;
                let rec = unsafe { &*rec_ptr };
                let rec_len = rec.RecordLength as usize;
                if rec_len == 0 {
                    break;
                }

                let name_offset = offset + rec.FileNameOffset as usize;
                let name_bytes_len = rec.FileNameLength as usize;

                if name_offset + name_bytes_len <= bytes_returned as usize {
                    let u16_slice = unsafe {
                        std::slice::from_raw_parts(
                            buffer[name_offset..].as_ptr() as *const u16,
                            name_bytes_len / 2,
                        )
                    };
                    let name = OsString::from_wide(u16_slice).to_string_lossy().to_string();
                    let is_dir = (rec.FileAttributes & 0x10) != 0; // FILE_ATTRIBUTE_DIRECTORY

                    callback(UsnRecord {
                        file_ref: rec.FileReferenceNumber,
                        parent_file_ref: rec.ParentFileReferenceNumber,
                        usn: rec.Usn,
                        reason: rec.Reason,
                        attributes: rec.FileAttributes,
                        name,
                        is_dir,
                    });
                }

                offset += rec_len;
            }
        }

        Ok(cursor)
    }

    /// Read incremental journal changes since `last_usn` using FSCTL_READ_USN_JOURNAL
    pub fn read_changes<F>(
        &self,
        journal_id: u64,
        last_usn: i64,
        mut callback: F,
    ) -> Result<i64, windows::core::Error>
    where
        F: FnMut(UsnRecord),
    {
        let read_data = READ_USN_JOURNAL_DATA_V0 {
            StartUsn: last_usn,
            ReasonMask: 0xFFFFFFFF,
            ReturnOnlyOnClose: 0,
            Timeout: 0,
            BytesToWaitFor: 0,
            UsnJournalID: journal_id,
        };

        let mut buffer = vec![0u8; 64 * 1024];
        let mut bytes_returned = 0u32;

        unsafe {
            DeviceIoControl(
                self.handle,
                FSCTL_READ_USN_JOURNAL,
                Some(&read_data as *const _ as *const _),
                std::mem::size_of::<READ_USN_JOURNAL_DATA_V0>() as u32,
                Some(buffer.as_mut_ptr() as *mut _),
                buffer.len() as u32,
                Some(&mut bytes_returned),
                None,
            )?;
        }

        if bytes_returned < 8 {
            return Ok(last_usn);
        }

        let next_usn = i64::from_le_bytes(buffer[0..8].try_into().unwrap());
        let mut offset = 8usize;

        while offset + std::mem::size_of::<USN_RECORD_V2>() <= bytes_returned as usize {
            let rec_ptr = buffer[offset..].as_ptr() as *const USN_RECORD_V2;
            let rec = unsafe { &*rec_ptr };
            let rec_len = rec.RecordLength as usize;
            if rec_len == 0 {
                break;
            }

            let name_offset = offset + rec.FileNameOffset as usize;
            let name_bytes_len = rec.FileNameLength as usize;

            if name_offset + name_bytes_len <= bytes_returned as usize {
                let u16_slice = unsafe {
                    std::slice::from_raw_parts(
                        buffer[name_offset..].as_ptr() as *const u16,
                        name_bytes_len / 2,
                    )
                };
                let name = OsString::from_wide(u16_slice).to_string_lossy().to_string();
                let is_dir = (rec.FileAttributes & 0x10) != 0;

                callback(UsnRecord {
                    file_ref: rec.FileReferenceNumber,
                    parent_file_ref: rec.ParentFileReferenceNumber,
                    usn: rec.Usn,
                    reason: rec.Reason,
                    attributes: rec.FileAttributes,
                    name,
                    is_dir,
                });
            }

            offset += rec_len;
        }

        Ok(next_usn)
    }
}
