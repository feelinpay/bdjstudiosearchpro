use crate::{decode_command, decode_event, encode_command, encode_event, IpcCommand, IpcEvent};
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

pub const DEFAULT_UNIX_SOCKET_PATH: &str = "/var/run/bdj_search_pro.sock";

pub struct UnixSocketServer {
    listener: UnixListener,
    path: PathBuf,
}

impl Drop for UnixSocketServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl UnixSocketServer {
    pub fn bind<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let p = path.as_ref().to_path_buf();
        if p.exists() {
            let _ = std::fs::remove_file(&p);
        }

        let listener = UnixListener::bind(&p)?;

        // Set 0660 permissions on socket (read/write for user and group)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o660));
        }

        Ok(Self { listener, path: p })
    }

    pub fn accept(&self) -> io::Result<UnixStream> {
        let (stream, _) = self.listener.accept()?;
        Ok(stream)
    }

    pub fn read_command(stream: &mut UnixStream) -> io::Result<IpcCommand> {
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf)?;
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload)?;
        decode_command(&payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
    }

    pub fn send_event(stream: &mut UnixStream, event: &IpcEvent) -> io::Result<()> {
        let payload = encode_event(event).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        let len_buf = (payload.len() as u32).to_le_bytes();
        stream.write_all(&len_buf)?;
        stream.write_all(&payload)?;
        stream.flush()?;
        Ok(())
    }
}

pub struct UnixSocketClient {
    stream: UnixStream,
}

impl UnixSocketClient {
    pub fn connect<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let stream = UnixStream::connect(path)?;
        Ok(Self { stream })
    }

    pub fn send_command(&mut self, cmd: &IpcCommand) -> io::Result<()> {
        let payload = encode_command(cmd).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        let len_buf = (payload.len() as u32).to_le_bytes();
        self.stream.write_all(&len_buf)?;
        self.stream.write_all(&payload)?;
        self.stream.flush()?;
        Ok(())
    }

    pub fn read_event(&mut self) -> io::Result<IpcEvent> {
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf)?;
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        self.stream.read_exact(&mut payload)?;
        decode_event(&payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
    }
}
