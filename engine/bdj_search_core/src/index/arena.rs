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

    /// Inserta un nombre en la arena y devuelve (desplazamiento, longitud, no_ascii).
    ///
    /// El límite es de 255 **bytes**, que es lo que cabe en la columna `name_len`.
    /// NTFS y APFS permiten 255 *unidades UTF-16*, que en UTF-8 pueden ocupar
    /// bastante más: un nombre largo con acentos, japonés o emoji supera los 255
    /// bytes con facilidad.
    ///
    /// Cortar por byte partiría una secuencia UTF-8 por la mitad, y entonces
    /// `get_name` devolvería `None` —el archivo desaparecería de toda búsqueda
    /// por texto, sin error ni aviso—. Por eso el corte retrocede hasta la
    /// frontera de carácter anterior.
    pub fn push(&mut self, name: &str) -> (u32, u8, bool) {
        let offset = self.data.len() as u32;
        let bytes = name.as_bytes();

        let mut len = bytes.len().min(255);
        while len > 0 && !name.is_char_boundary(len) {
            len -= 1;
        }

        let is_non_ascii = !name.is_ascii();
        self.data.extend_from_slice(&bytes[..len]);
        (offset, len as u8, is_non_ascii)
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

    #[test]
    fn nombre_largo_no_ascii_se_corta_por_frontera_de_caracter() {
        // 130 caracteres japoneses = 390 bytes: hay que cortar.
        let largo: String = "あ".repeat(130);
        let mut arena = NameArena::new();
        let (off, len, non_ascii) = arena.push(&largo);
        assert!(non_ascii);
        // 'あ' ocupa 3 bytes: el mayor múltiplo de 3 que cabe en 255 es 255.
        assert_eq!(len, 255);
        let recuperado = arena.get(off, len);
        assert!(
            recuperado.is_some(),
            "el nombre truncado debe seguir siendo UTF-8 válido"
        );
        assert_eq!(recuperado.unwrap().chars().count(), 85);
    }

    #[test]
    fn corte_a_mitad_de_secuencia_retrocede() {
        // 'ñ' ocupa 2 bytes. 128 'ñ' = 256 bytes: el corte en 255 caería a
        // mitad del último carácter y hay que retroceder a 254.
        let largo: String = "ñ".repeat(128);
        let mut arena = NameArena::new();
        let (off, len, _) = arena.push(&largo);
        assert_eq!(len, 254);
        assert_eq!(arena.get(off, len).unwrap().chars().count(), 127);
    }

    #[test]
    fn nombre_ascii_de_300_bytes_se_corta_en_255() {
        let largo = "a".repeat(300);
        let mut arena = NameArena::new();
        let (off, len, _) = arena.push(&largo);
        assert_eq!(len, 255);
        assert_eq!(arena.get(off, len).unwrap().len(), 255);
    }
}
