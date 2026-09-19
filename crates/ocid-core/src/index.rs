//! The on-disk JSON index: sha256 -> BlobRef and signed release records.
//!
//! This is plain files so both the daemon and `ocictl` can read it. The blob
//! *contents* live in the daemon's iroh-blobs store; an index entry for a
//! digest is only written once the blob has been imported/verified, so
//! "all blobs indexed" is a good proxy for "release complete".

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::{
    identity::PublisherId,
    oci::{self, Digest},
    paths::{write_atomic, Paths},
    release::{BlobRef, Referrer, Release},
};

#[derive(Debug, Clone)]
pub struct Index {
    paths: Paths,
}

impl Index {
    pub fn open(paths: &Paths) -> Result<Self> {
        paths.ensure_dirs()?;
        Ok(Self {
            paths: paths.clone(),
        })
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    // -----------------------------------------------------------------------
    // digests
    // -----------------------------------------------------------------------

    fn digest_path(&self, digest: &Digest) -> PathBuf {
        self.paths.digests().join(format!("{}.json", digest.hex()))
    }

    pub fn blob_ref(&self, digest: &Digest) -> Result<Option<BlobRef>> {
        let p = self.digest_path(digest);
        if !p.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&p)?;
        Ok(Some(serde_json::from_str(&text)?))
    }

    pub fn put_blob_ref(&self, r: &BlobRef) -> Result<()> {
        write_atomic(
            &self.digest_path(&r.digest),
            serde_json::to_vec_pretty(r)?.as_slice(),
        )
    }

    pub fn remove_blob_ref(&self, digest: &Digest) -> Result<bool> {
        let p = self.digest_path(digest);
        if p.exists() {
            std::fs::remove_file(&p)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Every indexed blob.
    pub fn list_blob_refs(&self) -> Result<Vec<BlobRef>> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(self.paths.digests()) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.extension().is_some_and(|e| e == "json") {
                    match std::fs::read_to_string(&p)
                        .context("read")
                        .and_then(|t| serde_json::from_str::<BlobRef>(&t).context("parse"))
                    {
                        Ok(r) => out.push(r),
                        Err(e) => {
                            tracing::warn!("skipping unreadable index entry {}: {e}", p.display())
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    /// True if every blob of the release has an index entry.
    pub fn is_indexed(&self, release: &Release) -> bool {
        release
            .all_blobs()
            .all(|b| self.digest_path(&b.digest).exists())
    }

    // -----------------------------------------------------------------------
    // referrers  (index/referrers/<subject-hex>/<referrer-hex>.json)
    // -----------------------------------------------------------------------

    fn referrers_dir(&self, subject: &Digest) -> PathBuf {
        self.paths.index().join("referrers").join(subject.hex())
    }

    pub fn put_referrer(&self, subject: &Digest, r: &Referrer) -> Result<()> {
        write_atomic(
            &self
                .referrers_dir(subject)
                .join(format!("{}.json", r.digest.hex())),
            serde_json::to_vec_pretty(r)?.as_slice(),
        )
    }

    pub fn list_referrers(&self, subject: &Digest) -> Result<Vec<Referrer>> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(self.referrers_dir(subject)) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.extension().is_some_and(|e| e == "json") {
                    if let Ok(text) = std::fs::read_to_string(&p) {
                        if let Ok(r) = serde_json::from_str::<Referrer>(&text) {
                            out.push(r);
                        }
                    }
                }
            }
        }
        out.sort_by(|a, b| a.digest.cmp(&b.digest));
        Ok(out)
    }

    /// Drop referrer records whose manifest blob is no longer indexed.
    pub fn prune_referrers(&self) -> Result<usize> {
        let mut n = 0;
        let root = self.paths.index().join("referrers");
        let Ok(rd) = std::fs::read_dir(&root) else {
            return Ok(0);
        };
        for subject_dir in rd.flatten() {
            let Ok(entries) = std::fs::read_dir(subject_dir.path()) else {
                continue;
            };
            for entry in entries.flatten() {
                let p = entry.path();
                let live = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .filter(|hex| {
                        hex.len() == 64
                            && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                    })
                    .map(|hex| self.paths.digests().join(format!("{hex}.json")).exists())
                    .unwrap_or(false);
                if !live {
                    let _ = std::fs::remove_file(&p);
                    n += 1;
                }
            }
            let _ = std::fs::remove_dir(subject_dir.path()); // only if empty
        }
        Ok(n)
    }

    // -----------------------------------------------------------------------
    // releases
    // -----------------------------------------------------------------------

    fn release_dir(&self, publisher: &PublisherId, name: &str) -> Result<PathBuf> {
        oci::validate_name(name)?;
        Ok(self.paths.releases().join(publisher.to_string()).join(name))
    }

    fn release_path(&self, publisher: &PublisherId, name: &str, tag: &str) -> Result<PathBuf> {
        oci::validate_tag(tag)?;
        Ok(self
            .release_dir(publisher, name)?
            .join(format!("{tag}.json")))
    }

    pub fn get_release(
        &self,
        publisher: &PublisherId,
        name: &str,
        tag: &str,
    ) -> Result<Option<Release>> {
        let p = self.release_path(publisher, name, tag)?;
        if !p.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&p)?;
        Ok(Some(serde_json::from_str(&text)?))
    }

    /// Store a release if it is new or supersedes the existing one.
    /// Returns `true` if written.
    pub fn put_release(&self, release: &Release) -> Result<bool> {
        let p = &release.payload;
        if let Some(existing) = self.get_release(&p.publisher, &p.name, &p.tag)? {
            if !release.supersedes(&existing) {
                return Ok(false);
            }
        }
        write_atomic(
            &self.release_path(&p.publisher, &p.name, &p.tag)?,
            serde_json::to_vec_pretty(release)?.as_slice(),
        )?;
        for r in &p.referrers {
            self.put_referrer(&p.manifest.digest, r)?;
        }
        Ok(true)
    }

    pub fn remove_release(&self, publisher: &PublisherId, name: &str, tag: &str) -> Result<bool> {
        let p = self.release_path(publisher, name, tag)?;
        if !p.exists() {
            return Ok(false);
        }
        std::fs::remove_file(&p)?;
        // prune now-empty directories up to the releases root
        let root = self.paths.releases();
        let mut dir = p.parent();
        while let Some(d) = dir {
            if d == root || std::fs::remove_dir(d).is_err() {
                break;
            }
            dir = d.parent();
        }
        Ok(true)
    }

    /// When the release record was last written (used for GC grace periods).
    pub fn release_mtime(&self, release: &Release) -> Option<std::time::SystemTime> {
        let p = &release.payload;
        self.release_path(&p.publisher, &p.name, &p.tag)
            .ok()
            .and_then(|p| std::fs::metadata(p).ok())
            .and_then(|m| m.modified().ok())
    }

    pub fn list_tags(&self, publisher: &PublisherId, name: &str) -> Result<Vec<String>> {
        let dir = self.release_dir(publisher, name)?;
        let mut tags = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for entry in rd.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().is_some_and(|e| e == "json") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        tags.push(stem.to_string());
                    }
                }
            }
        }
        tags.sort();
        Ok(tags)
    }

    /// All releases we hold, sorted by reference.
    pub fn list_releases(&self) -> Result<Vec<Release>> {
        let mut files = Vec::new();
        walk_json(&self.paths.releases(), &mut files)?;
        let mut out = Vec::with_capacity(files.len());
        for f in files {
            match std::fs::read_to_string(&f)
                .context("read")
                .and_then(|t| serde_json::from_str::<Release>(&t).context("parse"))
            {
                Ok(r) => out.push(r),
                Err(e) => tracing::warn!("skipping unreadable release {}: {e}", f.display()),
            }
        }
        out.sort_by_key(|r| r.reference());
        Ok(out)
    }
}

fn walk_json(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_json(&p, out)?;
        } else if p.extension().is_some_and(|e| e == "json")
            && !p
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.'))
        {
            out.push(p);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> Index {
        Index {
            paths: Paths {
                home: std::env::temp_dir()
                    .join(format!("ocid-index-test-{}", uuid::Uuid::new_v4())),
            },
        }
    }

    fn publisher() -> PublisherId {
        crate::identity::Identity::generate().id()
    }

    #[test]
    fn rejects_traversal_names() {
        let ix = index();
        let p = publisher();
        for name in [
            "../evil",
            "a/../b",
            "..",
            ".",
            "a/..",
            "/abs",
            "//abs",
            "a//b",
            "",
            "a/b/c/../../d",
        ] {
            assert!(
                ix.release_dir(&p, name).is_err(),
                "name {name:?} must be rejected"
            );
            assert!(
                ix.list_tags(&p, name).is_err(),
                "name {name:?} must be rejected by list_tags"
            );
        }
    }

    #[test]
    fn rejects_traversal_tags() {
        let ix = index();
        let p = publisher();
        for tag in ["../evil", "..", "", "a/b", "/abs", "a\\b", ".hidden"] {
            assert!(
                ix.release_path(&p, "app", tag).is_err(),
                "tag {tag:?} must be rejected"
            );
            assert!(
                ix.get_release(&p, "app", tag).is_err(),
                "tag {tag:?} must be rejected by get_release"
            );
            assert!(
                ix.remove_release(&p, "app", tag).is_err(),
                "tag {tag:?} must be rejected by remove_release"
            );
        }
    }

    #[test]
    fn valid_references_stay_within_the_store() {
        let ix = index();
        let p = publisher();
        let dir = ix.release_dir(&p, "web/app").unwrap();
        assert!(dir.starts_with(ix.paths.releases()));
        assert!(dir.ends_with(format!("{p}/web/app")));
        let file = ix.release_path(&p, "web/app", "1.0").unwrap();
        assert!(file.starts_with(ix.paths.releases()));
        assert!(file.ends_with("1.0.json"));
    }
}
