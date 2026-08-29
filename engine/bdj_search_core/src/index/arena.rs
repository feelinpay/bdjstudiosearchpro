#[derive(Debug, Default)]
pub struct NameArena {
    data: Vec<u8>,
}

impl NameArena {
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    pub fn push(&mut self, name: &str) -> (u32, u8) {
        let offset = self.data.len() as u32;
        let bytes = name.as_bytes();
        let len = bytes.len().min(255) as u8;
        self.data.extend_from_slice(&bytes[..len as usize]);
        (offset, len)
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}
