use once_cell::sync::Lazy;
use std::sync::{atomic, Arc};

fn gen_node() -> String {
    use rand::Rng;
    let mut b = [0; 6];
    rand::thread_rng().fill(&mut b[..]);
    base64::encode_config(b, base64::URL_SAFE_NO_PAD)
}

static NODE_ID: Lazy<Arc<str>> =
    Lazy::new(|| gen_node().into_boxed_str().into());

fn gen_cnt() -> u64 {
    static CNT_ID: atomic::AtomicU64 = atomic::AtomicU64::new(1);
    CNT_ID.fetch_add(1, atomic::Ordering::Relaxed)
}

/// Debugging unique identifier helper.
///
/// Construction via `Uniq::default()` will always result in the same
/// "node_id" unique to this process, as well as an incrementing counter.
///
/// Construction via `Uniq::fresh()` will result in a new random
/// "node_id" as well as an incrementing counter.
///
/// ```text
/// Uniq::default() -> "12MUNeh3:1"
/// Uniq::default() -> "12MUNeh3:2"
/// Uniq::fresh()   -> "RV1bMaCM:3"
/// Uniq::fresh()   -> "3psHcLKE:4"
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uniq(pub Arc<str>);

impl std::fmt::Display for Uniq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::ops::Deref for Uniq {
    type Target = Arc<str>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Default for Uniq {
    fn default() -> Self {
        Self(
            format!("{}:{}", *NODE_ID, gen_cnt())
                .into_boxed_str()
                .into(),
        )
    }
}

impl Uniq {
    /// Get a fresh node uniq.
    pub fn fresh() -> Self {
        Self(
            format!("{}:{}", gen_node(), gen_cnt())
                .into_boxed_str()
                .into(),
        )
    }

    /// Get a sub-uniq from this uniq.
    pub fn sub(&self) -> Self {
        Self(format!("{}:{}", self, gen_cnt()).into_boxed_str().into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanity() {
        let u1 = Uniq::default();
        let u2 = Uniq::default();
        let s1 = u2.sub();
        let s2 = u2.sub();
        let u3 = Uniq::fresh();

        println!("{u1} {u2} {s1} {s2} {u3}");

        assert_ne!(u1, u2);
        assert_ne!(u2, s1);
        assert_ne!(s1, s2);
        assert_ne!(u1, u3);
        assert_ne!(u2, u3);
    }
}
