//! The desktop's driver for the self-hosted backend (`flowie-server`): blocking HTTP, so it
//! runs on a plain thread beside the writer, with no async runtime (per-node-sync.md §5.1).
//!
//! The protocol, shared with the server:
//! - `POST {base}/v1/zones/{zone}/modify` with `{"saves": [Envelope]}` answers
//!   `{"results": [SaveResult]}`;
//! - `GET {base}/v1/zones/{zone}/changes?since=&limit=&wait=` answers a [`ChangeBatch`],
//!   holding the request up to `wait` seconds while nothing is new.
//!
//! Every request carries `Authorization: Bearer <device token>`.

use std::time::Duration;

use crate::engine::{Backend, ChangeBatch, SaveResult};
use crate::wire::{Envelope, ModifyRequest, ModifyResponse};

/// One outline (zone) on a self-hosted server.
pub struct HttpBackend {
    base: String,
    zone: String,
    token: String,
    agent: ureq::Agent,
    /// How long a pull may wait on the server for something new.
    pub wait: Duration,
}

impl HttpBackend {
    /// `base` is the server's URL (`https://sync.example.com`), `zone` the outline's name on
    /// it, `token` this device's token.
    pub fn new(base: &str, zone: &str, token: &str) -> Self {
        let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).timeout_read(Duration::from_secs(90)).build();
        HttpBackend {
            base: base.trim_end_matches('/').to_string(),
            zone: zone.to_string(),
            token: token.to_string(),
            agent,
            wait: Duration::ZERO,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/v1/zones/{}/{path}", self.base, self.zone)
    }

    fn auth(&self) -> String {
        format!("Bearer {}", self.token)
    }
}

fn describe(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(code, response) => {
            let body = response.into_string().unwrap_or_default();
            format!("the server answered {code}: {}", body.trim())
        }
        ureq::Error::Transport(t) => format!("couldn't reach the server: {t}"),
    }
}

impl Backend for HttpBackend {
    fn modify(&mut self, saves: Vec<Envelope>) -> Result<Vec<SaveResult>, String> {
        let response: ModifyResponse = self
            .agent
            .post(&self.url("modify"))
            .set("Authorization", &self.auth())
            .send_json(ModifyRequest { saves })
            .map_err(describe)?
            .into_json()
            .map_err(|e| format!("the server's answer didn't parse: {e}"))?;
        Ok(response.results)
    }

    fn changes(&mut self, cursor: Option<&str>, limit: usize) -> Result<ChangeBatch, String> {
        let mut request = self
            .agent
            .get(&self.url("changes"))
            .set("Authorization", &self.auth())
            .query("limit", &limit.to_string())
            .query("wait", &self.wait.as_secs().to_string());
        if let Some(cursor) = cursor {
            request = request.query("since", cursor);
        }
        request.call().map_err(describe)?.into_json().map_err(|e| format!("the server's answer didn't parse: {e}"))
    }
}
