use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct InternedExtensions {
    ext_to_id: HashMap<String, u16>,
    id_to_ext: Vec<String>,
}

impl InternedExtensions {
    pub fn new() -> Self {
        let mut s = Self {
            ext_to_id: HashMap::new(),
            id_to_ext: vec![String::new()], // ID 0 = no extension
        };
        // Pre-intern standard DJ audio and project extensions for predictable low IDs
        let common = [
            "wav", "mp3", "flac", "aiff", "aif", "m4a", "ogg", "opus", "wma", "alac", "ape", "aac",
            "als", "flp", "ptx", "cpr", "logicx", "rpp", "nki", "nkm", "cue", "m3u", "m3u8", "xml",
            "zip", "rar", "7z", "pdf", "docx", "mp4", "mov", "avi", "mkv", "jpg", "jpeg", "png",
        ];
        for ext in common {
            s.intern(ext);
        }
        s
    }

    pub fn intern(&mut self, ext: &str) -> u16 {
        let clean = ext.trim_start_matches('.').to_ascii_lowercase();
        if clean.is_empty() {
            return 0;
        }
        if let Some(&id) = self.ext_to_id.get(&clean) {
            return id;
        }
        let id = self.id_to_ext.len() as u16;
        self.ext_to_id.insert(clean.clone(), id);
        self.id_to_ext.push(clean);
        id
    }

    pub fn get_id(&self, ext: &str) -> Option<u16> {
        let clean = ext.trim_start_matches('.').to_ascii_lowercase();
        if clean.is_empty() {
            return Some(0);
        }
        self.ext_to_id.get(&clean).copied()
    }

    /// Cuántas extensiones distintas hay internadas.
    ///
    /// Ordenar por extensión necesita una tabla de rangos con una casilla por
    /// identificador; sin este dato habría que recorrer el índice entero para
    /// averiguar su tamaño.
    pub fn len(&self) -> usize {
        self.id_to_ext.len()
    }

    pub fn is_empty(&self) -> bool {
        self.id_to_ext.is_empty()
    }

    pub fn get_name(&self, id: u16) -> Option<&str> {
        self.id_to_ext.get(id as usize).map(|s| s.as_str())
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        // Number of extensions (excluding 0)
        let count = (self.id_to_ext.len() - 1) as u16;
        out.extend_from_slice(&count.to_le_bytes());
        for ext in &self.id_to_ext[1..] {
            let bytes = ext.as_bytes();
            let len = bytes.len() as u8;
            out.push(len);
            out.extend_from_slice(bytes);
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 2 {
            return None;
        }
        let count = u16::from_le_bytes([bytes[0], bytes[1]]) as usize;
        let mut s = Self {
            ext_to_id: HashMap::new(),
            id_to_ext: vec![String::new()],
        };
        let mut cursor = 2;
        for _ in 0..count {
            if cursor >= bytes.len() {
                return None;
            }
            let len = bytes[cursor] as usize;
            cursor += 1;
            if cursor + len > bytes.len() {
                return None;
            }
            let ext_str = std::str::from_utf8(&bytes[cursor..cursor + len]).ok()?;
            cursor += len;
            s.intern(ext_str);
        }
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interned_extensions() {
        let mut exts = InternedExtensions::new();
        let wav_id = exts.intern("WAV");
        let mp3_id = exts.intern(".mp3");
        assert_eq!(exts.intern("wav"), wav_id);
        assert_eq!(exts.get_name(wav_id), Some("wav"));
        assert_eq!(exts.get_name(mp3_id), Some("mp3"));

        let encoded = exts.encode();
        let decoded = InternedExtensions::decode(&encoded).unwrap();
        assert_eq!(decoded.get_id("wav"), Some(wav_id));
        assert_eq!(decoded.get_id("mp3"), Some(mp3_id));
    }
}
