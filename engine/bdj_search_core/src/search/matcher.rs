use memchr::memchr2;

#[derive(Clone, Debug)]
pub struct SubstringMatcher {
    pattern_lower: Vec<u8>,
    first_lower: u8,
    first_upper: u8,
    is_ascii: bool,
    /// El patrón ya en minúsculas, para la ruta Unicode.
    ///
    /// Antes se guardaba el patrón original y `matches_unicode` volvía a
    /// llamar a `to_lowercase()` **en cada entrada evaluada**, reservando una
    /// cadena por archivo con el patrón ya minusculizado al lado.
    pattern_lower_str: String,
}

impl SubstringMatcher {
    pub fn new(pattern: &str) -> Self {
        let pattern_lower = pattern.to_lowercase().into_bytes();
        let is_ascii = pattern.is_ascii();
        let first_lower = pattern_lower.first().copied().unwrap_or(0);
        let first_upper = if first_lower.is_ascii_lowercase() {
            first_lower.to_ascii_uppercase()
        } else {
            first_lower
        };

        let pattern_lower_str = pattern.to_lowercase();

        Self {
            pattern_lower,
            first_lower,
            first_upper,
            is_ascii,
            pattern_lower_str,
        }
    }

    #[inline(always)]
    pub fn matches(&self, text: &[u8], is_non_ascii: bool) -> bool {
        if self.pattern_lower.is_empty() {
            return true;
        }
        if is_non_ascii || !self.is_ascii {
            return self.matches_unicode(text);
        }

        self.matches_ascii(text)
    }

    #[inline(always)]
    fn matches_ascii(&self, text: &[u8]) -> bool {
        let plen = self.pattern_lower.len();
        if text.len() < plen {
            return false;
        }

        let mut offset = 0;
        let p_slice = &self.pattern_lower;

        while offset + plen <= text.len() {
            let hay = &text[offset..];
            if let Some(pos) = memchr2(self.first_lower, self.first_upper, hay) {
                let start = offset + pos;
                if start + plen > text.len() {
                    return false;
                }
                
                // Fast case-insensitive compare of the remainder
                let candidate = &text[start..start + plen];
                if Self::ascii_eq_ignore_case(candidate, p_slice) {
                    return true;
                }
                offset = start + 1;
            } else {
                return false;
            }
        }

        false
    }

    #[inline(always)]
    fn ascii_eq_ignore_case(a: &[u8], b_lower: &[u8]) -> bool {
        debug_assert_eq!(a.len(), b_lower.len());
        for i in 0..a.len() {
            if a[i].to_ascii_lowercase() != b_lower[i] {
                return false;
            }
        }
        true
    }

    fn matches_unicode(&self, text: &[u8]) -> bool {
        let Ok(s) = std::str::from_utf8(text) else {
            return false;
        };
        // Si el texto es ASCII puro no hace falta plegar mayúsculas con las
        // reglas de Unicode: basta la comparación rápida byte a byte.
        if s.is_ascii() && self.is_ascii {
            return self.matches_ascii(text);
        }
        s.to_lowercase().contains(self.pattern_lower_str.as_str())
    }

    /// El patrón en minúsculas, tal y como se compara.
    pub fn pattern(&self) -> &str {
        &self.pattern_lower_str
    }

    /// Los bytes del patrón, ya en minúsculas.
    #[inline(always)]
    pub fn pattern_bytes(&self) -> &[u8] {
        &self.pattern_lower
    }

    /// Primera letra del patrón, en minúscula y en mayúscula.
    ///
    /// Son las dos agujas que busca el barrido SIMD de la arena.
    #[inline(always)]
    pub fn first_bytes(&self) -> (u8, u8) {
        (self.first_lower, self.first_upper)
    }

    /// Cierto si el patrón es ASCII puro y admite la comparación rápida.
    #[inline(always)]
    pub fn is_ascii_pattern(&self) -> bool {
        self.is_ascii
    }

    /// Compara `text` con el patrón, byte a byte y sin distinguir mayúsculas.
    /// `text` debe medir exactamente lo que el patrón.
    #[inline(always)]
    pub fn eq_at(&self, text: &[u8]) -> bool {
        Self::ascii_eq_ignore_case(text, &self.pattern_lower)
    }
}

/// Coincidencia de comodines `*` y `?` sobre el **nombre completo**, sin
/// distinguir mayúsculas.
///
/// `*` equivale a cualquier secuencia (incluso vacía) y `?` a una sola letra.
/// A diferencia de `SubstringMatcher` (que busca una subcadena), aquí el patrón
/// debe cuadrar con todo el nombre, como en el explorador del sistema.
#[derive(Clone, Debug)]
pub struct WildcardMatcher {
    pattern_lower: String,
}

impl WildcardMatcher {
    /// Cierto si el patrón quiere coincidencia con comodines.
    #[inline(always)]
    pub fn contains_wildcard(pattern: &str) -> bool {
        pattern.contains('*') || pattern.contains('?')
    }

    pub fn new(pattern: &str) -> Self {
        Self {
            pattern_lower: pattern.to_lowercase(),
        }
    }

    /// Compara el nombre completo contra el patrón. Las consultas de comodín no
    /// están en la ruta caliente del barrido en bloque, así que no se optimiza
    /// a nivel de bytes: se pliega a minúsculas y se recorre con el autómata.
    #[inline]
    pub fn matches(&self, text: &[u8], _is_non_ascii: bool) -> bool {
        let Ok(name) = std::str::from_utf8(text) else {
            return false;
        };
        let hay = name.to_lowercase();
        let (pat, text) = (self.pattern_lower.as_bytes(), hay.as_bytes());
        let (mut pi, mut ti) = (0usize, 0usize);
        let (mut star, mut mark) = (None, 0usize);
        while ti < text.len() {
            if pi < pat.len() && (pat[pi] == b'?' || pat[pi] == text[ti]) {
                pi += 1;
                ti += 1;
            } else if pi < pat.len() && pat[pi] == b'*' {
                star = Some(pi);
                mark = ti;
                pi += 1;
            } else if let Some(s) = star {
                pi = s + 1;
                mark += 1;
                ti = mark;
            } else {
                return false;
            }
        }
        while pi < pat.len() && pat[pi] == b'*' {
            pi += 1;
        }
        pi == pat.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matcher_ascii() {
        let matcher = SubstringMatcher::new("michael");
        assert!(matcher.matches(b"Michael Jackson - Billie Jean.mp3", false));
        assert!(matcher.matches(b"MICHAEL.wav", false));
        assert!(!matcher.matches(b"Madonna.flac", false));
    }

    #[test]
    fn test_matcher_unicode() {
        let matcher = SubstringMatcher::new("canción");
        assert!(matcher.matches("Mi CANCIÓN Favorita.wav".as_bytes(), true));
        assert!(!matcher.matches("Otra cosa.wav".as_bytes(), true));
    }

    #[test]
    fn test_wildcard_estrella() {
        let m = WildcardMatcher::new("michael*");
        assert!(m.matches(b"Michael Jackson - Billie Jean.mp3", false));
        assert!(m.matches(b"MICHAEL.wav", false));
        assert!(!m.matches(b"Madonna.flac", false));

        let sufijo = WildcardMatcher::new("*jackson");
        assert!(sufijo.matches(b"Michael Jackson", false));
        assert!(!sufijo.matches(b"Michael Jordan", false));
    }

    #[test]
    fn test_wildcard_interrogacion() {
        let m = WildcardMatcher::new("??a?");
        assert!(m.matches(b"Flac", false));
        assert!(m.matches(b"FLAC", false));
        assert!(!m.matches(b"Flacx", false));
    }

    #[test]
    fn test_wildcard_extension_y_prefijo() {
        let m_ext = WildcardMatcher::new("*.mp3");
        assert!(m_ext.matches(b"track_01.mp3", false));
        assert!(m_ext.matches(b"TRACK_01.MP3", false));
        assert!(!m_ext.matches(b"track_01.wav", false));

        let m_kick = WildcardMatcher::new("Kick*.wav");
        assert!(m_kick.matches(b"Kick_909_punch.wav", false));
        assert!(m_kick.matches(b"kick.wav", false));
        assert!(m_kick.matches(b"KICK_SUB.WAV", false));
        assert!(!m_kick.matches(b"Snare_909.wav", false));
    }
}
