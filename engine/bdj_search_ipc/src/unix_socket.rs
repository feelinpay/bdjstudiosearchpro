use crate::{decode_command, decode_event, encode_command, encode_event, IpcCommand, IpcEvent};
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

pub const DEFAULT_UNIX_SOCKET_PATH: &str = "/var/run/bdj_search_pro.sock";

/// Ruta del socket de control resuelta en tiempo de ejecución.
///
/// En macOS el socket vive preferentemente en la carpeta del sistema
/// `/Library/Application Support/BDJ Studio/Search Pro/bdj_search_pro.sock`
/// para que tanto el LaunchDaemon (root) como la app de usuario (Flutter)
/// compartan el mismo canal de comunicación. Si esa carpeta no es escribible
/// (por ejemplo, en desarrollo local), cae a la carpeta de usuario o `/var/run`.
pub fn default_unix_socket_path() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        let system_dir = PathBuf::from("/Library/Application Support/BDJ Studio/Search Pro");
        let system_sock = system_dir.join("bdj_search_pro.sock");
        if system_dir.exists() || system_sock.exists() {
            return system_sock;
        }
        if let Ok(home) = std::env::var("HOME") {
            let dir = PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("BDJ Studio")
                .join("Search Pro");
            let _ = std::fs::create_dir_all(&dir);
            return dir.join("bdj_search_pro.sock");
        }
    }
    PathBuf::from(DEFAULT_UNIX_SOCKET_PATH)
}

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

        // Permisos restringidos según contexto:
        // - Root LaunchDaemon: grupo 'staff' (GID 20) y permisos 0660 (solo root y usuarios de consola).
        // - Usuario estándar: permisos 0600 (solo el usuario actual).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            #[cfg(target_os = "macos")]
            unsafe {
                let uid = libc::getuid();
                if uid == 0 {
                    if let Ok(c_path) = std::ffi::CString::new(p.to_string_lossy().as_bytes()) {
                        libc::chown(c_path.as_ptr(), 0, 20);
                    }
                    let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o660));
                } else {
                    let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
                }
            }
            #[cfg(all(unix, not(target_os = "macos")))]
            {
                let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
            }
        }

        Ok(Self { listener, path: p })
    }

    pub fn accept(&self) -> io::Result<UnixStream> {
        let (stream, _) = self.listener.accept()?;
        #[cfg(unix)]
        {
            Self::verify_peer(&stream)?;
        }
        Ok(stream)
    }

    #[cfg(target_os = "macos")]
    fn console_user_uid() -> Option<libc::uid_t> {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata("/dev/console").ok().map(|m| m.uid())
    }

    #[cfg(target_os = "macos")]
    fn get_peer_creds(stream: &UnixStream) -> io::Result<(libc::uid_t, libc::gid_t)> {
        use std::os::unix::io::AsRawFd;
        let mut euid: libc::uid_t = 0;
        let mut egid: libc::gid_t = 0;
        let res = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut euid, &mut egid) };
        if res != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((euid, egid))
    }

    #[cfg(target_os = "linux")]
    fn get_peer_creds(stream: &UnixStream) -> io::Result<(libc::uid_t, libc::gid_t)> {
        use std::os::unix::io::AsRawFd;
        let mut ucred = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let res = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut ucred as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if res != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((ucred.uid, ucred.gid))
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn get_peer_creds(_stream: &UnixStream) -> io::Result<(libc::uid_t, libc::gid_t)> {
        Ok((unsafe { libc::getuid() }, unsafe { libc::getgid() }))
    }

    #[cfg(unix)]
    fn verify_peer(stream: &UnixStream) -> io::Result<()> {
        let (euid, _egid) = Self::get_peer_creds(stream)?;
        let my_uid = unsafe { libc::getuid() };

        #[cfg(target_os = "macos")]
        {
            if my_uid == 0 {
                // Root daemon: solo permitir a root (0) o al usuario conectado activamente a la consola gráfica (/dev/console)
                let active_console = Self::console_user_uid();
                if euid == 0 || active_console.is_some_and(|uid| uid == euid) {
                    Ok(())
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "Conexión IPC no autorizada: el cliente no es el usuario de consola activo",
                    ))
                }
            } else if euid == my_uid || euid == 0 {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "UID del cliente IPC no coincide",
                ))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            if my_uid == 0 || euid == my_uid || euid == 0 {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "UID del cliente IPC no coincide",
                ))
            }
        }
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
