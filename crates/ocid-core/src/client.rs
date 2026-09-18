//! HTTP client for the daemon's `/_ocid/` control API.

use std::{net::SocketAddr, time::Duration};

use anyhow::{anyhow, bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;

#[derive(Clone)]
pub struct Client {
    base: String,
    http: reqwest::Client,
}

impl Client {
    pub fn new(listen: SocketAddr) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        Self {
            base: format!("http://{listen}"),
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(3))
                .build()
                .unwrap_or_default(),
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
