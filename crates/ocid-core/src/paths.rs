//! On-disk layout of an ocid node.
//!
//! ```text
//! $OCID_HOME (default: XDG data home — ~/.local/share/ocid;
//!             a pre-existing legacy ~/.ocid keeps being used)
//! ├── secret.key                 ed25519 secret key (hex, mode 0600)
//! ├── config.toml                node configuration
//! ├── policy.toml                seeding policy: seeds, follows, aliases
//! ├── peers.json                 known peer addresses (bootstrap)
//! ├── dns-pins.json              TOFU pins for DNS publisher records
//! ├── tls/                       self-signed CA + server cert (tls = "auto")
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
    /// Resolve the node home: explicit `home`, else `$OCID_HOME`, else the
    /// XDG data home (see [`Paths::from_env`]).
    pub fn resolve(home: Option<PathBuf>) -> Result<Self> {
        if let Some(h) = home {
            return Ok(Self { home: h });
        }
        Self::from_env()
    }

    /// Resolve the node home from `$OCID_HOME`; otherwise the XDG data home
    /// (`$XDG_DATA_HOME/ocid`, default `~/.local/share/ocid`) when it already
    /// exists, then a pre-existing legacy `~/.ocid`; fresh nodes default to
    /// the XDG home. The extension's daemon pod bind-mounts that same XDG
    /// home, so `ocictl`/`ocitop` and the pod share one node with no env
    /// wiring.
    pub fn from_env() -> Result<Self> {
        if let Some(v) = std::env::var_os("OCID_HOME") {
            if !v.is_empty() {
                return Ok(Self {
                    home: PathBuf::from(v),
                });
            }
        }
        let home_dir =
            dirs::home_dir().context("could not determine home directory; set OCID_HOME")?;
        let xdg = std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir.join(".local").join("share"))
            .join("ocid");
        let legacy = home_dir.join(".ocid");
        Ok(Self {
            home: choose_home(&xdg, &legacy),
        })
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
    /// TOFU pins for DNS publisher records (see `dns.rs`).
    pub fn dns_pins(&self) -> PathBuf {
        self.home.join("dns-pins.json")
    }
    pub fn tls_dir(&self) -> PathBuf {
        self.home.join("tls")
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

/// Home-selection rule (pure so it can be tested): an existing XDG home
/// wins, then an existing legacy home; fresh systems start in the XDG home.
fn choose_home(xdg: &Path, legacy: &Path) -> PathBuf {
    if xdg.exists() {
        xdg.to_path_buf()
    } else if legacy.exists() {
        legacy.to_path_buf()
    } else {
        xdg.to_path_buf()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ocid-paths-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn absent(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ocid-paths-{tag}-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn xdg_home_wins_when_both_exist() {
        let (xdg, legacy) = (tmpdir("xdg"), tmpdir("legacy"));
        assert_eq!(choose_home(&xdg, &legacy), xdg);
    }

    #[test]
    fn legacy_home_used_when_xdg_absent() {
        let (xdg, legacy) = (absent("xdg"), tmpdir("legacy"));
        assert_eq!(choose_home(&xdg, &legacy), legacy);
    }

    #[test]
    fn fresh_systems_default_to_xdg() {
        let (xdg, legacy) = (absent("xdg"), absent("legacy"));
        assert_eq!(choose_home(&xdg, &legacy), xdg);
    }
}
