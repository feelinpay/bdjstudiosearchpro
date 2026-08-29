use crate::{decode_command, decode_event, encode_command, encode_event, IpcCommand, IpcEvent};
use std::io;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows::Win32::Security::SECURITY_ATTRIBUTES;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING,
    FILE_FLAGS_AND_ATTRIBUTES,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe,
    PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

const SDDL_REVISION_1: u32 = 1;
const PIPE_ACCESS_DUPLEX_VAL: u32 = 0x00000003;

pub struct PipeServer {
    handle: HANDLE,
    pipe_name: String,
}

impl Drop for PipeServer {
    fn drop(&mut self) {
        if self.handle != INVALID_HANDLE_VALUE && self.handle != HANDLE(std::ptr::null_mut()) {
            unsafe {
                let _ = DisconnectNamedPipe(self.handle);
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

impl PipeServer {
    /// Creates a Named Pipe instance with DACL allowing Authenticated Users (AU)
    /// and Administrators/SYSTEM full access.
    pub fn create(pipe_name: &str) -> io::Result<Self> {
        let wide_name: Vec<u16> = pipe_name.encode_utf16().chain(std::iter::once(0)).collect();

        // SDDL string:
        // D: (DACL)
        // (A;;GRGW;;;AU) - Allow Generic Read/Write to Authenticated Users
        // (A;;GA;;;BA)   - Allow Generic All to Builtin Administrators
        // (A;;GA;;;SY)   - Allow Generic All to Local System
        let sddl = "D:(A;;GRGW;;;AU)(A;;GA;;;BA)(A;;GA;;;SY)\0";
        let wide_sddl: Vec<u16> = sddl.encode_utf16().collect();

        let mut sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            bInheritHandle: false.into(),
            ..Default::default()
        };

        let mut p_sd = windows::Win32::Security::PSECURITY_DESCRIPTOR(std::ptr::null_mut());
        let sd_ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(wide_sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut p_sd,
                None,
            )
        };

        if sd_ok.is_ok() {
            sa.lpSecurityDescriptor = p_sd.0;
        }

        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(wide_name.as_ptr()),
                FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX_VAL),
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                65536,
                65536,
                0,
                Some(&sa),
            )
        };

        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        Ok(Self {
            handle,
            pipe_name: pipe_name.to_string(),
        })
    }

    pub fn pipe_name(&self) -> &str {
        &self.pipe_name
    }

    /// Waits for a client process to connect to the named pipe.
    pub fn wait_for_client(&self) -> io::Result<()> {
        let res = unsafe { ConnectNamedPipe(self.handle, None) };
        if let Err(e) = res {
            // ERROR_PIPE_CONNECTED (535) is expected if client already connected between calls
            if e.code().0 != 0x80070217u32 as i32 && e.code().0 != 535 {
                return Err(io::Error::from_raw_os_error(e.code().0));
            }
        }
        Ok(())
    }

    pub fn disconnect(&self) {
        unsafe {
            let _ = FlushFileBuffers(self.handle);
            let _ = DisconnectNamedPipe(self.handle);
        }
    }

    /// Read incoming IpcCommand (4-byte length prefix + postcard payload)
    pub fn read_command(&self) -> io::Result<IpcCommand> {
        let mut len_buf = [0u8; 4];
        let mut bytes_read = 0u32;
        unsafe {
            ReadFile(
                self.handle,
                Some(&mut len_buf),
                Some(&mut bytes_read),
                None,
            )?;
        }
        if bytes_read < 4 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Incomplete length prefix"));
        }
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        let mut total = 0usize;
        while total < len {
            let mut read = 0u32;
            unsafe {
                ReadFile(
                    self.handle,
                    Some(&mut payload[total..]),
                    Some(&mut read),
                    None,
                )?;
            }
            if read == 0 {
                break;
            }
            total += read as usize;
        }

        decode_command(&payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
    }

    /// Send outgoing IpcEvent (4-byte length prefix + postcard payload)
    pub fn send_event(&self, event: &IpcEvent) -> io::Result<()> {
        let payload = encode_event(event).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        let len_buf = (payload.len() as u32).to_le_bytes();
        let mut written = 0u32;
        unsafe {
            WriteFile(self.handle, Some(&len_buf), Some(&mut written), None)?;
            WriteFile(self.handle, Some(&payload), Some(&mut written), None)?;
        }
        Ok(())
    }
}

pub struct PipeClient {
    handle: HANDLE,
}

impl Drop for PipeClient {
    fn drop(&mut self) {
        if self.handle != INVALID_HANDLE_VALUE && self.handle != HANDLE(std::ptr::null_mut()) {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

impl PipeClient {
    pub fn connect(pipe_name: &str) -> io::Result<Self> {
        let wide_name: Vec<u16> = pipe_name.encode_utf16().chain(std::iter::once(0)).collect();

        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide_name.as_ptr()),
                (GENERIC_READ | GENERIC_WRITE).0,
                windows::Win32::Storage::FileSystem::FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )?
        };

        Ok(Self { handle })
    }

    pub fn send_command(&self, cmd: &IpcCommand) -> io::Result<()> {
        let payload = encode_command(cmd).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        let len_buf = (payload.len() as u32).to_le_bytes();
        let mut written = 0u32;
        unsafe {
            WriteFile(self.handle, Some(&len_buf), Some(&mut written), None)?;
            WriteFile(self.handle, Some(&payload), Some(&mut written), None)?;
        }
        Ok(())
    }

    pub fn read_event(&self) -> io::Result<IpcEvent> {
        let mut len_buf = [0u8; 4];
        let mut bytes_read = 0u32;
        unsafe {
            ReadFile(
                self.handle,
                Some(&mut len_buf),
                Some(&mut bytes_read),
                None,
            )?;
        }
        if bytes_read < 4 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Incomplete length prefix"));
        }
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        let mut total = 0usize;
        while total < len {
            let mut read = 0u32;
            unsafe {
                ReadFile(
                    self.handle,
                    Some(&mut payload[total..]),
                    Some(&mut read),
                    None,
                )?;
            }
            if read == 0 {
                break;
            }
            total += read as usize;
        }

        decode_event(&payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
    }
}
