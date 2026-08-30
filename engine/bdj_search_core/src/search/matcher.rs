use memchr::memchr2;

#[derive(Clone, Debug)]
pub struct SubstringMatcher {
    pattern_lower: Vec<u8>,
    first_lower: u8,
    first_upper: u8,
    is_ascii: bool,
    pattern_str: String,
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

        Self {
            pattern_lower,
            first_lower,
            first_upper,
            is_ascii,
            pattern_str: pattern.to_string(),
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
        if let Ok(s) = std::str::from_utf8(text) {
            let s_lower = s.to_lowercase();
            let pat_lower = self.pattern_str.to_lowercase();
            s_lower.contains(&pat_lower)
        } else {
            false
        }
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
}
