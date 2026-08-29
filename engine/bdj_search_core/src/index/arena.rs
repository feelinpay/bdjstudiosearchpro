#[derive(Debug, Default, Clone)]
pub struct NameArena {
    data: Vec<u8>,
}

impl NameArena {
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: Vec::with_capacity(capacity),
        }
    }

    /// Pushes a name into the arena, returning (offset, len, is_non_ascii).
    /// Enforces max 255 bytes per name (standard NTFS and APFS component limit).
    pub fn push(&mut self, name: &str) -> (u32, u8, bool) {
        let offset = self.data.len() as u32;
        let bytes = name.as_bytes();
        let len = bytes.len().min(255) as u8;
        let slice = &bytes[..len as usize];
        
        let is_non_ascii = !name.is_ascii();
        self.data.extend_from_slice(slice);
        (offset, len, is_non_ascii)
    }

    pub fn get(&self, offset: u32, len: u8) -> Option<&str> {
        let start = offset as usize;
        let end = start + (len as usize);
        if end <= self.data.len() {
            std::str::from_utf8(&self.data[start..end]).ok()
        } else {
            None
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name_arena() {
        let mut arena = NameArena::new();
        let (off1, len1, non_ascii1) = arena.push("Billie Jean.mp3");
        assert_eq!(off1, 0);
        assert_eq!(len1, 15);
        assert!(!non_ascii1);

        let (off2, len2, non_ascii2) = arena.push("Canción de Cuna.wav");
        assert_eq!(off2, 15);
        assert!(non_ascii2);

        assert_eq!(arena.get(off1, len1), Some("Billie Jean.mp3"));
        assert_eq!(arena.get(off2, len2), Some("Canción de Cuna.wav"));
    }
}
