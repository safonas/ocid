//! A BLAKE3 hash as used by iroh-blobs, without depending on iroh-blobs.
//!
//! Serialises as lowercase hex in human-readable formats (JSON/TOML) and as
//! raw bytes otherwise (postcard) — identical to `iroh_blobs::Hash`, so the
//! two are wire- and disk-compatible.

use std::{fmt, str::FromStr};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Blake3([u8; 32]);

impl Blake3 {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// First 10 hex chars, for logs.
    pub fn fmt_short(&self) -> String {
        hex::encode(&self.0[..5])
    }
}

impl fmt::Display for Blake3 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Blake3 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Blake3({})", self.fmt_short())
    }
}

impl FromStr for Blake3 {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        let bytes: [u8; 32] = hex::decode(s)
            .map_err(|e| anyhow!("invalid blake3 hex: {e}"))?
            .try_into()
            .map_err(|_| anyhow!("blake3 hash must be 32 bytes"))?;
        Ok(Self(bytes))
    }
}

impl Serialize for Blake3 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if s.is_human_readable() {
            s.serialize_str(&self.to_hex())
        } else {
            self.0.serialize(s)
        }
    }
}

impl<'de> Deserialize<'de> for Blake3 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        if d.is_human_readable() {
            let s = String::deserialize(d)?;
            s.parse().map_err(serde::de::Error::custom)
        } else {
            let bytes = <[u8; 32]>::deserialize(d)?;
            Ok(Self(bytes))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let h = Blake3::from_bytes([7u8; 32]);
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(json, format!("\"{}\"", "07".repeat(32)));
        let back: Blake3 = serde_json::from_str(&json).unwrap();
        assert_eq!(back, h);
        assert_eq!(h.to_string().parse::<Blake3>().unwrap(), h);
    }
}
