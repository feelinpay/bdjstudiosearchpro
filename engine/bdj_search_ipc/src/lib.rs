use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub enum IpcCommand {
    EnableIndexing,
    DisableIndexing,
    SetVolumeIndexed { volume_id: u8, indexed: bool },
    AddFolder { path: String },
    RescanVolume { volume_id: u8 },
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub enum IpcEvent {
    IndexUpdated { generation: u64, entry_count: u64 },
    IndexingProgress {
        volume_id: u8,
        entries_scanned: u64,
        is_done: bool,
    },
    VolumeStateChanged { volume_id: u8, is_mounted: bool },
}

pub const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\BDJSearchProPipe";

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
    fn test_codec_roundtrip() {
        let cmd = IpcCommand::SetVolumeIndexed {
            volume_id: 1,
            indexed: true,
        };
        let bytes = encode_command(&cmd).unwrap();
        let decoded = decode_command(&bytes).unwrap();
        assert_eq!(cmd, decoded);
    }
}
