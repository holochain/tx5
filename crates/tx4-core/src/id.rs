use crate::*;

/// Tx4 32-byte identifier.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id(pub [u8; 32]);

impl serde::Serialize for Id {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_b64())
    }
}

impl<'de> serde::Deserialize<'de> for Id {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>
    {
        let tmp = String::deserialize(deserializer)?;
        Self::from_b64(&tmp).map_err(serde::de::Error::custom)
    }
}

impl std::fmt::Debug for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut a = self.to_b64();
        a.replace_range(8..a.len() - 8, "..");
        f.write_str(&a)
    }
}

impl std::fmt::Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_b64())
    }
}

impl std::ops::Deref for Id {
    type Target = [u8; 32];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<[u8]> for Id {
    fn as_ref(&self) -> &[u8] {
        &self.0[..]
    }
}

impl Id {
    /// Load from a slice.
    pub fn from_slice(s: &[u8]) -> Result<Self> {
        if s.len() != 32 {
            return Err(Error::id("InvalidIdLength"));
        }
        let mut out = [0; 32];
        out.copy_from_slice(s);
        Ok(Self(out))
    }

    /// Decode a base64 encoded Id.
    pub fn from_b64(s: &str) -> Result<Self> {
        let v = base64::decode_config(s, base64::URL_SAFE_NO_PAD)
            .map_err(Error::err)?;
        Self::from_slice(&v)
    }

    /// Encode a Id as base64.
    pub fn to_b64(&self) -> String {
        base64::encode_config(self, base64::URL_SAFE_NO_PAD)
    }
}
