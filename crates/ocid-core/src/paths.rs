//! On-disk layout of an ocid node.
//!
//! ```text
//! $OCID_HOME (default ~/.ocid)
//! ├── secret.key                 ed25519 secret key (hex, mode 0600)
//! ├── config.toml                node configuration
//! ├── policy.toml                seeding policy: seeds, follows, aliases
//! ├── peers.json                 known peer addresses (bootstrap)
//! ├── blobs/                     iroh-blobs FsStore (BLAKE3 content-addressed)
//! ├── index/
//! │   ├── digests/<sha256>.json  sha256 -> {blake3, size}
//! │   └── releases/<publisher>/<name>/<tag>.json   signed release records
//! └── uploads/                   in-progress registry uploads
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Paths {
    pub home: PathBuf,
}

impl Paths {
    /// Resolve the node home: explicit `home`, else `$OCID_HOME`, else `~/.ocid`.
    pub fn resolve(home: Option<PathBuf>) -> Result<Self> {
        if let Some(h) = home {
            return Ok(Self { home: h });
        }
        Self::from_env()
    }

    /// Resolve the node home from `$OCID_HOME`, falling back to `~/.ocid`.
    pub fn from_env() -> Result<Self> {
        let home = match std::env::var_os("OCID_HOME") {
            Some(v) if !v.is_empty() => PathBuf::from(v),
            _ => dirs::home_dir()
                .context("could not determine home directory; set OCID_HOME")?
                .join(".ocid"),
        };
        Ok(Self { home })
    }

    pub fn secret_key(&self) -> PathBuf {
        self.home.join("secret.key")
    }
    pub fn config(&self) -> PathBuf {
        self.home.join("config.toml")
    }
    pub fn policy(&self) -> PathBuf {
        self.home.join("policy.toml")
    }
    pub fn peers(&self) -> PathBuf {
        self.home.join("peers.json")
    }
    pub fn blobs(&self) -> PathBuf {
        self.home.join("blobs")
    }
    pub fn index(&self) -> PathBuf {
        self.home.join("index")
    }
    pub fn digests(&self) -> PathBuf {
        self.index().join("digests")
    }
    pub fn releases(&self) -> PathBuf {
        self.index().join("releases")
    }
    pub fn uploads(&self) -> PathBuf {
        self.home.join("uploads")
    }

    pub fn is_initialized(&self) -> bool {
        self.secret_key().exists()
    }

    /// Create all directories.
    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [
            &self.home,
            &self.blobs(),
            &self.digests(),
            &self.releases(),
            &self.uploads(),
        ] {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        Ok(())
    }
}

/// Atomically write a file (write to temp + rename).
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path.parent().context("path has no parent")?;
    std::fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("file"),
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&tmp, data).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming to {}", path.display()))?;
    Ok(())
}
