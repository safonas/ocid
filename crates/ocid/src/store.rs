//! Local content store: an iroh-blobs `FsStore` (BLAKE3-addressed, verified
//! streaming) plus the shared on-disk index (sha256 -> BLAKE3, releases).

use std::{ops::Deref, path::Path, time::Duration};

use anyhow::{bail, Context, Result};
use bytes::Bytes;
use iroh_blobs::{
    api::{blobs::BlobReader, TempTag},
    store::{
        fs::{options::Options, FsStore},
        GcConfig,
    },
    Hash,
};
use n0_future::StreamExt;
use ocid_core::{
    hash::Blake3,
    index::Index,
    oci::Digest,
    paths::Paths,
    release::{BlobRef, Release},
};
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncReadExt;

/// Convert between the core hash type and iroh-blobs' hash.
pub trait HashExt {
    fn to_iroh(&self) -> Hash;
}
impl HashExt for Blake3 {
    fn to_iroh(&self) -> Hash {
        Hash::from_bytes(*self.as_bytes())
    }
}
pub fn from_iroh(h: Hash) -> Blake3 {
    Blake3::from_bytes(*h.as_bytes())
}

#[derive(Debug, Clone)]
pub struct Store {
    index: Index,
    blobs: FsStore,
}

/// `Store` derefs to the index so `store.get_release(..)` etc. just work.
impl Deref for Store {
    type Target = Index;
    fn deref(&self) -> &Index {
        &self.index
    }
}

impl Store {
    /// Open the store. `blob_gc_interval` is how often iroh-blobs runs its own
    /// mark & sweep, deleting data that no tag (pin) or temp tag protects.
    pub async fn open(paths: &Paths, blob_gc_interval: Duration) -> Result<Self> {
        let index = Index::open(paths)?;
        let root = paths.blobs();
        let mut options = Options::new(&root);
        options.gc = Some(GcConfig {
            interval: blob_gc_interval,
            add_protected: None,
        });
        let blobs = FsStore::load_with_opts(root.join("blobs.db"), options)
            .await
            .with_context(|| format!("opening blob store at {}", root.display()))?;
        Ok(Self { index, blobs })
    }

    /// The underlying iroh-blobs API (for the protocol handler and downloader).
    pub fn blobs(&self) -> &iroh_blobs::api::Store {
        &self.blobs
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.blobs.shutdown().await?;
        Ok(())
    }

    fn tag_name(digest: &Digest) -> String {
        format!("blob:{}", digest.hex())
    }

    pub async fn has_hash(&self, hash: Blake3) -> Result<bool> {
        Ok(self.blobs.blobs().has(hash.to_iroh()).await?)
    }

    /// Record an index entry and pin the blob with a named tag.
    pub async fn record_blob(&self, r: &BlobRef) -> Result<()> {
        self.blobs
            .tags()
            .set(Self::tag_name(&r.digest), r.hash.to_iroh())
            .await?;
        self.index.put_blob_ref(r)
    }

    /// Import in-memory data. If `expect` is given the sha256 must match.
    pub async fn put_blob_bytes(
        &self,
        data: Bytes,
        expect: Option<&Digest>,
        media_type: Option<String>,
    ) -> Result<BlobRef> {
        let digest = Digest::sha256(&data);
        if let Some(e) = expect {
            if e != &digest {
                bail!("digest mismatch: expected {e}, got {digest}");
            }
        }
        let size = data.len() as u64;
        let tag = self
            .blobs
            .add_bytes(data)
            .with_named_tag(Self::tag_name(&digest))
            .await?;
        let r = BlobRef {
            digest,
            hash: from_iroh(tag.hash),
            size,
            media_type,
        };
        self.index.put_blob_ref(&r)?;
        Ok(r)
    }

    /// Import a file (copied into the store). The file is removed afterwards.
    pub async fn put_blob_file(
        &self,
        path: &Path,
        expect: Option<&Digest>,
        media_type: Option<String>,
    ) -> Result<BlobRef> {
        let (digest, size) = sha256_file(path).await?;
        if let Some(e) = expect {
            if e != &digest {
                bail!("digest mismatch: expected {e}, got {digest}");
            }
        }
        let tag = self
            .blobs
            .add_path(path)
            .with_named_tag(Self::tag_name(&digest))
            .await?;
        let _ = tokio::fs::remove_file(path).await;
        let r = BlobRef {
            digest,
            hash: from_iroh(tag.hash),
            size,
            media_type,
        };
        self.index.put_blob_ref(&r)?;
        Ok(r)
    }

    pub async fn read_hash(&self, hash: Blake3) -> Result<Bytes> {
        Ok(self.blobs.get_bytes(hash.to_iroh()).await?)
    }

    pub fn blob_reader(&self, hash: Blake3) -> BlobReader {
        self.blobs.blobs().reader(hash.to_iroh())
    }

    /// Verify that the blob stored under `r.hash` really has sha256 `r.digest`
    /// and size `r.size`. Used after a p2p download, before indexing.
    pub async fn verify_blob(&self, r: &BlobRef) -> Result<()> {
        let mut reader = self.blob_reader(r.hash);
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        let mut total = 0u64;
        loop {
            let n = reader.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            total += n as u64;
        }
        let digest = Digest::from_sha256_bytes(&hasher.finalize());
        if digest != r.digest {
            bail!(
                "blob {} has sha256 {digest}, expected {}",
                r.hash.fmt_short(),
                r.digest
            );
        }
        if total != r.size {
            bail!("blob {} has size {total}, expected {}", r.digest, r.size);
        }
        Ok(())
    }

    /// Drop the pin (named tag) of a blob. The data is removed by the next
    /// [`Self::sweep`].
    pub async fn unpin_blob(&self, digest: &Digest) -> Result<()> {
        self.blobs.tags().delete(Self::tag_name(digest)).await?;
        Ok(())
    }

    /// All `blob:<sha256>` pins: (digest, hash).
    pub async fn list_pins(&self) -> Result<Vec<(Digest, Blake3)>> {
        let mut out = Vec::new();
        let mut stream = self.blobs.tags().list().await?;
        while let Some(item) = stream.next().await {
            let info = item?;
            let name: Bytes = info.name.into();
            if let Some(hex) = std::str::from_utf8(&name)
                .ok()
                .and_then(|n| n.strip_prefix("blob:"))
            {
                if let Ok(d) = format!("sha256:{hex}").parse::<Digest>() {
                    out.push((d, from_iroh(info.hash)));
                }
            }
        }
        Ok(out)
    }

    /// Protect a hash from the store's GC until the returned tag is dropped.
    /// Used while a blob is downloaded, verified and pinned.
    pub async fn protect(&self, hash: Blake3) -> Result<TempTag> {
        Ok(self.blobs.tags().temp_tag(hash.to_iroh()).await?)
    }

    /// True if the manifest and every blob of the release are present.
    pub async fn is_complete(&self, release: &Release) -> Result<bool> {
        for b in release.all_blobs() {
            if !self.has_hash(b.hash).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

pub async fn sha256_file(path: &Path) -> Result<(Digest, u64)> {
    let mut f = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut total = 0u64;
    loop {
        let n = f.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    Ok((Digest::from_sha256_bytes(&hasher.finalize()), total))
}
