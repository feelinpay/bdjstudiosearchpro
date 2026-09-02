use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VolumeRecord {
    pub id: u8,
    pub mount_prefix: String,
    pub label: String,
    pub fs_type: String,
    pub is_connected: bool,
    #[serde(default)]
    pub last_usn_or_fsevent: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct VolumeTable {
    pub volumes: Vec<VolumeRecord>,
}

impl VolumeTable {
    pub fn new() -> Self {
        Self { volumes: Vec::new() }
    }

    pub fn add_or_update(
        &mut self,
        mount_prefix: &str,
        label: &str,
        fs_type: &str,
        is_connected: bool,
    ) -> u8 {
        if let Some(vol) = self.volumes.iter_mut().find(|v| v.mount_prefix == mount_prefix) {
            vol.label = label.to_string();
            vol.fs_type = fs_type.to_string();
            vol.is_connected = is_connected;
            return vol.id;
        }

        let id = self.volumes.len() as u8;
        self.volumes.push(VolumeRecord {
            id,
            mount_prefix: mount_prefix.to_string(),
            label: label.to_string(),
            fs_type: fs_type.to_string(),
            is_connected,
            last_usn_or_fsevent: 0,
        });
        id
    }

    pub fn update_cursor(&mut self, mount_prefix: &str, cursor: u64) {
        if let Some(vol) = self.volumes.iter_mut().find(|v| v.mount_prefix == mount_prefix) {
            vol.last_usn_or_fsevent = cursor;
        }
    }

    pub fn get_cursor(&self, mount_prefix: &str) -> u64 {
        self.volumes
            .iter()
            .find(|v| v.mount_prefix == mount_prefix)
            .map(|v| v.last_usn_or_fsevent)
            .unwrap_or(0)
    }

    pub fn get(&self, id: u8) -> Option<&VolumeRecord> {
        self.volumes.get(id as usize)
    }

    pub fn encode(&self) -> Vec<u8> {
        postcard::to_allocvec(self).unwrap_or_default()
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        postcard::from_bytes(bytes).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_volume_table_roundtrip() {
        let mut table = VolumeTable::new();
        let c_id = table.add_or_update("C:\\", "Sistema", "NTFS", true);
        let usb_id = table.add_or_update("E:\\", "USB Cabina", "exFAT", false);
        table.update_cursor("C:\\", 123_456_789);

        let encoded = table.encode();
        let decoded = VolumeTable::decode(&encoded).unwrap();

        assert_eq!(decoded.get(c_id).unwrap().label, "Sistema");
        assert_eq!(decoded.get_cursor("C:\\"), 123_456_789);
        assert!(!decoded.get(usb_id).unwrap().is_connected);
    }
}
