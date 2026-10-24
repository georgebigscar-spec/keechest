//! Random password generation.

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::random;

pub const LOWERCASE: &str = "abcdefghijklmnopqrstuvwxyz";
pub const UPPERCASE: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
pub const DIGITS: &str = "0123456789";
pub const SYMBOLS: &str = "!#$%&()*+,-./:;<=>?@[]^_{|}~";
/// Characters that are easy to mix up when read or typed by hand.
pub const AMBIGUOUS: &str = "Il1O0o|";

pub const MIN_LENGTH: usize = 4;
pub const MAX_LENGTH: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneratorOptions {
    pub length: usize,
    pub lowercase: bool,
    pub uppercase: bool,
    pub digits: bool,
    pub symbols: bool,
    pub exclude_ambiguous: bool,
}

impl Default for GeneratorOptions {
    fn default() -> Self {
        Self {
            length: 20,
            lowercase: true,
            uppercase: true,
            digits: true,
            symbols: true,
            exclude_ambiguous: false,
        }
    }
}

impl GeneratorOptions {
    /// The enabled character classes, ambiguous characters removed if asked.
    fn classes(&self) -> Vec<Vec<u8>> {
        [
            (self.lowercase, LOWERCASE),
            (self.uppercase, UPPERCASE),
            (self.digits, DIGITS),
            (self.symbols, SYMBOLS),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .map(|(_, set)| {
            set.bytes()
                .filter(|b| !(self.exclude_ambiguous && AMBIGUOUS.as_bytes().contains(b)))
                .collect()
        })
        .collect()
    }

    fn length(&self) -> usize {
        self.length.clamp(MIN_LENGTH, MAX_LENGTH)
    }

    /// Approximate strength of passwords from these options, in bits.
    pub fn entropy_bits(&self) -> f64 {
        let alphabet: usize = self.classes().iter().map(Vec::len).sum();
        if alphabet == 0 {
            return 0.0;
        }
        self.length() as f64 * (alphabet as f64).log2()
    }

    /// A new random password with at least one character of every enabled
    /// class. `None` when no class is enabled.
    pub fn generate(&self) -> Option<Zeroizing<String>> {
        let classes = self.classes();
        if classes.is_empty() {
            return None;
        }
        let alphabet: Vec<u8> = classes.concat();
        let length = self.length().max(classes.len());
        let pick = |set: &[u8]| set[random::below(set.len() as u32) as usize];

        let mut buf = Zeroizing::new(Vec::with_capacity(length));
        for class in &classes {
            buf.push(pick(class));
        }
        while buf.len() < length {
            buf.push(pick(&alphabet));
        }
        // Fisher-Yates, so the guaranteed characters are not at the front.
        for i in (1..buf.len()).rev() {
            let j = random::below(i as u32 + 1) as usize;
            buf.swap(i, j);
        }
        // All ASCII; from_utf8 reuses the buffer rather than copying it.
        let bytes = std::mem::take(&mut *buf);
        Some(Zeroizing::new(String::from_utf8(bytes).expect("ASCII")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_and_classes() {
        let opts = GeneratorOptions::default();
        for _ in 0..200 {
            let p = opts.generate().unwrap();
            assert_eq!(p.len(), 20);
            assert!(p.bytes().any(|b| b.is_ascii_lowercase()));
            assert!(p.bytes().any(|b| b.is_ascii_uppercase()));
            assert!(p.bytes().any(|b| b.is_ascii_digit()));
            assert!(p.bytes().any(|b| SYMBOLS.as_bytes().contains(&b)));
        }
    }

    #[test]
    fn only_selected_and_unambiguous() {
        let opts = GeneratorOptions {
            length: 64,
            lowercase: false,
            uppercase: true,
            digits: true,
            symbols: false,
            exclude_ambiguous: true,
        };
        for _ in 0..100 {
            let p = opts.generate().unwrap();
            assert!(
                p.bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            );
            assert!(!p.bytes().any(|b| AMBIGUOUS.as_bytes().contains(&b)));
        }
    }

    #[test]
    fn clamps_and_none() {
        let mut opts = GeneratorOptions {
            length: 1,
            ..Default::default()
        };
        assert_eq!(opts.generate().unwrap().len(), MIN_LENGTH);
        opts.length = 10_000;
        assert_eq!(opts.generate().unwrap().len(), MAX_LENGTH);
        let none = GeneratorOptions {
            lowercase: false,
            uppercase: false,
            digits: false,
            symbols: false,
            ..Default::default()
        };
        assert!(none.generate().is_none());
        assert_eq!(none.entropy_bits(), 0.0);
    }

    #[test]
    fn entropy() {
        let digits = GeneratorOptions {
            length: 10,
            lowercase: false,
            uppercase: false,
            digits: true,
            symbols: false,
            exclude_ambiguous: false,
        };
        assert!((digits.entropy_bits() - 10.0 * 10f64.log2()).abs() < 1e-9);
        assert!(GeneratorOptions::default().entropy_bits() > 120.0);
    }

    #[test]
    fn passwords_differ() {
        let opts = GeneratorOptions::default();
        assert_ne!(*opts.generate().unwrap(), *opts.generate().unwrap());
    }
}
