//! Password strength estimation with zxcvbn, which looks for dictionary
//! words, keyboard patterns, dates and repeats rather than counting
//! character classes.

/// zxcvbn gets slow on long inputs, and past this length any password that
/// is not a pattern is strong anyway.
const MAX_CHECKED: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Strength {
    /// 0 (trivial) to 4 (strong), zxcvbn's score.
    pub score: u8,
    /// log2 of the estimated number of guesses.
    pub bits: u32,
}

/// Estimates how hard `password` is to guess. `user_inputs` are words an
/// attacker would try first, like the entry's title and username.
pub fn estimate(password: &str, user_inputs: &[&str]) -> Strength {
    if password.is_empty() {
        return Strength { score: 0, bits: 0 };
    }
    let checked: String = password.chars().take(MAX_CHECKED).collect();
    let checked = zeroize::Zeroizing::new(checked);
    let entropy = zxcvbn::zxcvbn(&checked, user_inputs);
    Strength {
        score: u8::from(entropy.score()),
        bits: entropy
            .guesses_log10()
            .mul_add(std::f64::consts::LOG2_10, 0.0) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scores() {
        assert_eq!(estimate("", &[]).score, 0);
        assert!(estimate("password", &[]).score <= 1);
        assert!(estimate("qwerty123", &[]).score <= 1);
        let strong = estimate("u8#Kq!w2Zr@9vLm$", &[]);
        assert_eq!(strong.score, 4);
        assert!(strong.bits > 50);
        // The username makes a password containing it weaker.
        let with = estimate("georgebigscar", &["georgebigscar"]);
        let without = estimate("georgebigscar", &[]);
        assert!(with.bits < without.bits);
    }
}
