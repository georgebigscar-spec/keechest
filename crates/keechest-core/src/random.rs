//! Randomness from the operating system. Nothing here uses a userspace PRNG.

/// Fills `buf` from the OS random source.
///
/// Panics if the OS has no usable random source, since nothing secure can
/// be done without one.
pub fn fill(buf: &mut [u8]) {
    getrandom::fill(buf).expect("the OS random number generator is unavailable");
}

pub fn array<const N: usize>() -> [u8; N] {
    let mut buf = [0; N];
    fill(&mut buf);
    buf
}

fn next_u32() -> u32 {
    u32::from_le_bytes(array())
}

/// A uniform integer in `0..n`, without modulo bias.
pub fn below(n: u32) -> u32 {
    assert!(n > 0, "empty range");
    // Reject the top partial copy of 0..n so every residue is equally likely.
    let zone = (1u64 << 32) - (1u64 << 32) % u64::from(n);
    loop {
        let x = u64::from(next_u32());
        if x < zone {
            return (x % u64::from(n)) as u32;
        }
    }
}

/// A random id, 16 bytes as lowercase hex.
pub fn id() -> String {
    array::<16>().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn below_stays_in_range_and_covers_it() {
        let mut seen = [0u32; 7];
        for _ in 0..7000 {
            seen[below(7) as usize] += 1;
        }
        // Each bucket expects 1000; this only catches gross bias.
        assert!(seen.iter().all(|&n| (700..1300).contains(&n)), "{seen:?}");
        assert_eq!(below(1), 0);
    }

    #[test]
    fn ids_are_unique_hex() {
        let a = id();
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(a, id());
    }
}
