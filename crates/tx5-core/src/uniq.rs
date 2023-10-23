use std::sync::atomic;

/// Debugging unique identifier helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uniq(pub u64);

impl std::fmt::Display for Uniq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl Default for Uniq {
    fn default() -> Self {
        static CNT_ID: atomic::AtomicU64 = atomic::AtomicU64::new(1);
        Self(CNT_ID.fetch_add(1, atomic::Ordering::Relaxed))
    }
}

impl Uniq {
    /// Get a sub-uniq from this uniq.
    pub fn sub(&self) -> SubUniq {
        SubUniq(self.0, Uniq::default().0)
    }
}

/// Debugging unique identifier helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SubUniq(pub u64, pub u64);

impl std::fmt::Display for SubUniq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)?;
        f.write_str(":")?;
        self.1.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanity() {
        let u1 = Uniq::default();
        let u2 = Uniq::default();

        println!("{u1} {u2}");

        assert_ne!(u1, u2);
    }
}
