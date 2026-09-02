use serde::{Deserialize, Serialize};

/// Un cambio que la **propia aplicación** acaba de hacer en el disco.
///
/// Se anuncia al servicio para dos cosas a la vez: que el índice lo refleje al
/// instante, sin esperar al diario del sistema de archivos, y que el evento que
/// llegará por ese diario un instante después se descarte en lugar de aplicarse
/// por segunda vez.
#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub enum LocalChange {
    Created { path: String, is_dir: bool },
    Removed { path: String },
    Renamed { from: String, to: String },
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub enum IpcCommand {
    EnableIndexing,
    DisableIndexing,
    /// Excluye o vuelve a incluir un volumen, por su prefijo de montaje
    /// (`C:\`, `/Volumes/USB`). Se usa el prefijo y no el identificador
    /// numérico porque este último cambia al recompactar el índice.
    SetVolumeIndexed {
        mount_prefix: String,
        indexed: bool,
    },
    /// Añade una carpeta suelta al índice, esté donde esté.
    AddFolder {
        path: String,
    },
    RemoveFolder {
        path: String,
    },
    /// Vuelve a escanear un volumen desde cero.
    RescanVolume {
        mount_prefix: String,
    },
    /// Cambios hechos por la aplicación. Véase [`LocalChange`].
    LocalChanges {
        changes: Vec<LocalChange>,
    },
    /// Estado de la licencia según la aplicación.
    ///
    /// Sin licencia activa el servicio deja de indexar y no publica nada nuevo.
    /// La comprobación criptográfica de verdad la hace la aplicación con la
    /// clave pública del ecosistema; el servicio se limita a obedecer, porque la
    /// tubería solo la pueden abrir los usuarios de esta máquina.
    SetLicenseState {
        active: bool,
        expires_at: u64,
    },
    /// Pide el estado actual. La respuesta es [`IpcEvent::Status`].
    GetStatus,
}

/// Estado de un volumen tal y como lo ve el servicio.
#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct VolumeStatus {
    pub mount_prefix: String,
    pub label: String,
    pub fs_type: String,
    pub is_connected: bool,
    pub is_indexed: bool,
    pub entry_count: u64,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub enum IpcEvent {
    IndexUpdated {
        generation: u64,
        entry_count: u64,
    },
    IndexingProgress {
        volume_id: u8,
        entries_scanned: u64,
        is_done: bool,
    },
    VolumeStateChanged {
        volume_id: u8,
        is_mounted: bool,
    },
    Status {
        indexing_enabled: bool,
        license_active: bool,
        generation: u64,
        entry_count: u64,
        volumes: Vec<VolumeStatus>,
    },
    /// La orden se recibió y se aplicó.
    Ack,
    /// La orden no se pudo aplicar, con el motivo.
    Error {
        message: String,
    },
}

pub const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\BDJSearchProPipe";

#[cfg(windows)]
pub mod windows_pipe;

#[cfg(windows)]
pub use windows_pipe::{PipeClient, PipeServer};

#[cfg(unix)]
pub mod unix_socket;

#[cfg(unix)]
pub use unix_socket::{default_unix_socket_path, DEFAULT_UNIX_SOCKET_PATH, UnixSocketClient, UnixSocketServer};

pub fn encode_command(cmd: &IpcCommand) -> Result<Vec<u8>, postcard::Error> {
    postcard::to_allocvec(cmd)
}

pub fn decode_command(bytes: &[u8]) -> Result<IpcCommand, postcard::Error> {
    postcard::from_bytes(bytes)
}

pub fn encode_event(evt: &IpcEvent) -> Result<Vec<u8>, postcard::Error> {
    postcard::to_allocvec(evt)
}

pub fn decode_event(bytes: &[u8]) -> Result<IpcEvent, postcard::Error> {
    postcard::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ida_y_vuelta_de_las_ordenes() {
        let ordenes = [
            IpcCommand::EnableIndexing,
            IpcCommand::DisableIndexing,
            IpcCommand::SetVolumeIndexed {
                mount_prefix: "D:\\".into(),
                indexed: false,
            },
            IpcCommand::AddFolder {
                path: "C:\\Musica".into(),
            },
            IpcCommand::RemoveFolder {
                path: "C:\\Musica".into(),
            },
            IpcCommand::RescanVolume {
                mount_prefix: "C:\\".into(),
            },
            IpcCommand::SetLicenseState {
                active: true,
                expires_at: 1_800_000_000,
            },
            IpcCommand::GetStatus,
            IpcCommand::LocalChanges {
                changes: vec![
                    LocalChange::Created {
                        path: "C:\\a.wav".into(),
                        is_dir: false,
                    },
                    LocalChange::Removed {
                        path: "C:\\b.wav".into(),
                    },
                    LocalChange::Renamed {
                        from: "C:\\c.wav".into(),
                        to: "C:\\d.wav".into(),
                    },
                ],
            },
        ];

        for cmd in ordenes {
            let bytes = encode_command(&cmd).unwrap();
            assert_eq!(decode_command(&bytes).unwrap(), cmd, "falló con {cmd:?}");
        }
    }

    #[test]
    fn ida_y_vuelta_de_los_eventos() {
        let evento = IpcEvent::Status {
            indexing_enabled: true,
            license_active: true,
            generation: 7,
            entry_count: 1_234_567,
            volumes: vec![VolumeStatus {
                mount_prefix: "C:\\".into(),
                label: "Sistema".into(),
                fs_type: "NTFS".into(),
                is_connected: true,
                is_indexed: true,
                entry_count: 1_234_567,
            }],
        };
        let bytes = encode_event(&evento).unwrap();
        assert_eq!(decode_event(&bytes).unwrap(), evento);
    }
}
