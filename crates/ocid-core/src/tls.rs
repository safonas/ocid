//! Self-signed TLS material for the registry, generated on first start when
//! `config.toml` says `tls = "auto"`.
//!
//! One private CA (`ca.crt` / `ca.key`) signs the server certificate
//! (`server.crt` / `server.key`). The CA is what clients install to trust
//! the registry: podman's `certs.d`, curl's `--cacert`, and the ocid
//! clients (`ocictl`, `ocitop`) all read `tls/ca.crt`.
//!
//! The set is regenerated wholesale when any file is missing or empty —
//! private keys are not recoverable, so a partial set cannot be completed;
//! clients that pinned the old CA must re-install the new one.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};

use crate::paths::write_atomic;

pub const CA_CERT_FILE: &str = "ca.crt";
pub const CA_KEY_FILE: &str = "ca.key";
pub const SERVER_CERT_FILE: &str = "server.crt";
pub const SERVER_KEY_FILE: &str = "server.key";

/// Hostnames and IPs the server certificate answers to. Loopback covers
/// same-host clients; `host.containers.internal` covers podman-machine VMs
/// reaching a registry bound on the host.
const SERVER_SANS: &[&str] = &["localhost", "host.containers.internal", "127.0.0.1", "::1"];

/// The node's TLS files (existing or freshly generated).
#[derive(Debug, Clone)]
pub struct TlsMaterial {
    pub ca_cert: PathBuf,
    pub server_cert: PathBuf,
    pub server_key: PathBuf,
}

impl TlsMaterial {
    /// PEM bytes of the CA certificate, for clients to trust.
    pub fn ca_cert_pem(&self) -> Result<Vec<u8>> {
        fs::read(&self.ca_cert).with_context(|| format!("reading {}", self.ca_cert.display()))
    }
}

/// Return the node's TLS files, generating a fresh CA + server certificate
/// if any of them is missing. Idempotent: an existing complete set is
/// reused as-is.
pub fn ensure_material(dir: &Path) -> Result<TlsMaterial> {
    let material = TlsMaterial {
        ca_cert: dir.join(CA_CERT_FILE),
        server_cert: dir.join(SERVER_CERT_FILE),
        server_key: dir.join(SERVER_KEY_FILE),
    };
    let complete = fs::metadata(&material.ca_cert).is_ok_and(|m| m.len() > 0)
        && fs::metadata(dir.join(CA_KEY_FILE)).is_ok_and(|m| m.len() > 0)
        && fs::metadata(&material.server_cert).is_ok_and(|m| m.len() > 0)
        && fs::metadata(&material.server_key).is_ok_and(|m| m.len() > 0);
    if complete {
        return Ok(material);
    }
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

    // The CA signs the server certificate; it is the part clients install.
    let ca_key = KeyPair::generate()?;
    let ca_key_pem = ca_key.serialize_pem();
    let mut ca_params = CertificateParams::new(Vec::<String>::new())?;
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "ocid local CA");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_cert = ca_params.self_signed(&ca_key)?;
    let ca_issuer = Issuer::new(ca_params, ca_key);

    let server_key = KeyPair::generate()?;
    let mut params = CertificateParams::new(
        SERVER_SANS
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
    )?;
    params.distinguished_name.push(DnType::CommonName, "ocid");
    params.key_usages.push(KeyUsagePurpose::DigitalSignature);
    params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ServerAuth);
    let server_cert = params.signed_by(&server_key, &ca_issuer)?;

    write_private(&dir.join(CA_KEY_FILE), ca_key_pem.as_bytes())?;
    write_atomic(&material.ca_cert, ca_cert.pem().as_bytes())?;
    write_private(&material.server_key, server_key.serialize_pem().as_bytes())?;
    write_atomic(&material.server_cert, server_cert.pem().as_bytes())?;
    tracing::info!("generated TLS material in {}", dir.display());
    Ok(material)
}

/// Atomic write, then lock the file down (it holds a private key).
fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    write_atomic(path, data)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("chmod {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("ocid-tls-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn generates_and_reuses_material() {
        let dir = tmpdir();
        let m = ensure_material(&dir).unwrap();
        for p in [&m.ca_cert, &m.server_cert, &m.server_key] {
            assert!(p.exists(), "{} missing", p.display());
        }
        assert_eq!(m.ca_cert, dir.join(CA_CERT_FILE));
        let ca = m.ca_cert_pem().unwrap();
        assert!(ca.starts_with(b"-----BEGIN CERTIFICATE-----"));
        // idempotent: an existing set is reused verbatim
        let m2 = ensure_material(&dir).unwrap();
        assert_eq!(ca, m2.ca_cert_pem().unwrap());
        // an incomplete set cannot be completed: everything is regenerated
        fs::remove_file(&m.server_cert).unwrap();
        let m3 = ensure_material(&dir).unwrap();
        assert_ne!(ca, m3.ca_cert_pem().unwrap());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_files_are_private() {
        let dir = tmpdir();
        let m = ensure_material(&dir).unwrap();
        for p in [dir.join(CA_KEY_FILE), m.server_key] {
            let mode = fs::metadata(&p).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "{} not 0600", p.display());
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
