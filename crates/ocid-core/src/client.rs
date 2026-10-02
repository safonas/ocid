//! HTTP client for the daemon's `/_ocid/` control API.

use std::{
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::Path,
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::{config::Config, paths::Paths, tls::CA_CERT_FILE};

#[derive(Clone)]
pub struct Client {
    base: String,
    http: reqwest::Client,
}

/// Where to dial: a daemon listening on all interfaces (the container
/// default, `OCID_LISTEN=0.0.0.0:5050`) is reached on loopback — connecting
/// to the unspecified address fails TLS name verification (no SAN for
/// `0.0.0.0`), so it is normalized to `127.0.0.1`.
fn dial_addr(listen: SocketAddr) -> SocketAddr {
    if listen.ip().is_unspecified() {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), listen.port())
    } else {
        listen
    }
}

impl Client {
    pub fn new(listen: SocketAddr) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listen = dial_addr(listen);
        Self {
            base: format!("http://{listen}"),
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(3))
                .build()
                .unwrap_or_default(),
        }
    }

    /// A client for a daemon per its persisted `config.toml`: `https`,
    /// trusting the daemon's own CA, when `tls = "auto"`, plain `http`
    /// otherwise.
    pub fn from_config(config: &Config, paths: &Paths) -> Self {
        match config.tls {
            crate::config::TlsMode::Off => Self::new(config.listen),
            crate::config::TlsMode::Auto => {
                Self::https(config.listen, &paths.tls_dir().join(CA_CERT_FILE))
            }
        }
    }

    /// An `https` client that additionally trusts the daemon's CA
    /// certificate (unreadable CA material falls through: the request then
    /// fails verification, which is the honest signal).
    fn https(listen: SocketAddr, ca_cert: &Path) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listen = dial_addr(listen);
        let mut builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(3));
        match fs::read(ca_cert)
            .map_err(anyhow::Error::from)
            .and_then(|pem| reqwest::Certificate::from_pem(&pem).map_err(anyhow::Error::from))
        {
            Ok(cert) => builder = builder.tls_certs_merge([cert]),
            Err(e) => {
                tracing::warn!("reading CA certificate {}: {e}", ca_cert.display())
            }
        }
        Self {
            base: format!("https://{listen}"),
            http: builder.build().unwrap_or_default(),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    pub async fn is_running(&self) -> bool {
        self.http
            .get(format!("{}/_ocid/status", self.base))
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .is_ok()
    }

    async fn handle<T: DeserializeOwned>(
        &self,
        resp: reqwest::Result<reqwest::Response>,
    ) -> Result<T> {
        let resp = resp.map_err(|e| {
            if e.is_connect() {
                anyhow!(
                    "ocid daemon is not running at {} (start it with `ocid`)",
                    self.base
                )
            } else {
                anyhow!("{e}")
            }
        })?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            let msg = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from))
                .unwrap_or(text);
            bail!("{msg}");
        }
        serde_json::from_str(&text).with_context(|| format!("decoding response: {text}"))
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.handle(
            self.http
                .get(format!("{}{path}", self.base))
                .timeout(Duration::from_secs(5))
                .send()
                .await,
        )
        .await
    }

    pub async fn post<T: DeserializeOwned>(&self, path: &str, body: &impl Serialize) -> Result<T> {
        self.handle(
            self.http
                .post(format!("{}{path}", self.base))
                .json(body)
                .timeout(Duration::from_secs(60))
                .send()
                .await,
        )
        .await
    }

    /// Open streaming connection to SSE event endpoint.
    pub async fn events(&self) -> Result<reqwest::Response> {
        let resp = self
            .http
            .get(format!("{}/_ocid/events", self.base))
            .send()
            .await
            .map_err(|e| anyhow!("connecting to event stream: {e}"))?;
        if !resp.status().is_success() {
            bail!("event stream returned HTTP {}", resp.status());
        }
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unspecified_listen_dials_loopback() {
        // container default: OCID_LISTEN=0.0.0.0:5050
        let c = Client::new("0.0.0.0:5050".parse().unwrap());
        assert_eq!(c.base_url(), "http://127.0.0.1:5050");
        let c = Client::new("127.0.0.1:5051".parse().unwrap());
        assert_eq!(c.base_url(), "http://127.0.0.1:5051");
    }
}
