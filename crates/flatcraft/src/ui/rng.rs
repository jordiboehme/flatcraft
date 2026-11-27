//! A tiny seeded random number generator (xorshift64) for the easter eggs:
//! the maker quote of the About dialog and the rare "Punch it." toast.
//! Tests seed it, so every roll is known in advance.

/// The seed of a new app. `main` seeds from the clock instead.
pub const DEFAULT_SEED: u64 = 0x5EED_F1A7_C4AF_7001;

/// xorshift64 (Marsaglia 2003). Not for anything that matters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XorShift(u64);

impl XorShift {
    /// A generator with this seed. Zero would stay zero forever, so it is
    /// replaced by [`DEFAULT_SEED`].
    pub fn new(seed: u64) -> Self {
        Self(if seed == 0 { DEFAULT_SEED } else { seed })
    }

    /// A seed from the system clock, for the real app.
    pub fn clock_seed() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(DEFAULT_SEED, |d| d.as_nanos() as u64)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A number in `0..n` (`n` at least 1).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }

    /// True about one time in `n`.
    pub fn one_in(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }
}

impl Default for XorShift {
    fn default() -> Self {
        Self::new(DEFAULT_SEED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_seed_does_not_get_stuck() {
        let mut r = XorShift::new(0);
        assert_ne!(r.next_u64(), 0);
    }

    #[test]
    fn one_in_five_is_about_one_in_five() {
        let mut r = XorShift::new(42);
        let hits = (0..10_000).filter(|_| r.one_in(5)).count();
        assert!((1_700..2_300).contains(&hits), "{hits}");
    }

    #[test]
    fn same_seed_same_rolls() {
        let a: Vec<u64> = {
            let mut r = XorShift::new(7);
            (0..5).map(|_| r.next_u64()).collect()
        };
        let mut r = XorShift::new(7);
        assert_eq!(a, (0..5).map(|_| r.next_u64()).collect::<Vec<_>>());
    }
}
