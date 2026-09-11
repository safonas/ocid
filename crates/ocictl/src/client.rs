//! HTTP client for the daemon's `/_ocid/` control API.

use std::net::SocketAddr;

use anyhow::{anyhow, bail, Context, Result};
use serde::de::DeserializeOwned;

pub struct Client {
    base: String,
    http: reqwest::Client,
}

impl Client {
    pub fn new(listen: SocketAddr) -> Self {
        Self {
            base: format!("http://{listen}"),
            http: reqwest::Client::new(),
        }
    }

    pub async fn is_running(&self) -> bool {
        self.http
            .get(format!("{}/_ocid/status", self.base))
            .timeout(std::time::Duration::from_secs(2))
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
        self.handle(self.http.get(format!("{}{path}", self.base)).send().await)
            .await
    }

    pub async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<T> {
        self.handle(
            self.http
                .post(format!("{}{path}", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
}
