use crate::ansi::clean_log_tail;
use crate::error::PetError;
use crate::model::{decode_agents, AgentSnapshot};

pub struct Client {
    base: String,
    token: Option<String>,
    http: reqwest::blocking::Client,
}

impl Client {
    pub fn new(base: String, token: Option<String>) -> Self {
        let http = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        Client {
            base: base.trim_end_matches('/').to_string(),
            token,
            http,
        }
    }

    fn auth(&self, rb: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
        match &self.token {
            Some(t) => rb.bearer_auth(t),
            None => rb,
        }
    }

    pub fn get_agents(&self) -> Result<Vec<AgentSnapshot>, PetError> {
        let url = format!("{}/agents", self.base);
        let resp = self
            .auth(self.http.get(&url))
            .send()
            .map_err(|_| PetError::Transport)?;
        let code = resp.status().as_u16();
        if !resp.status().is_success() {
            return Err(PetError::from_status(code));
        }
        let body = resp.text().map_err(|_| PetError::Transport)?;
        decode_agents(&body).map_err(|_| PetError::Decode)
    }

    pub fn send_input(&self, id: &str, text: &str) -> Result<(), PetError> {
        let url = format!("{}/agents/{}/input", self.base, id);
        let resp = self
            .auth(self.http.post(&url).json(&serde_json::json!({ "text": text })))
            .send()
            .map_err(|_| PetError::Transport)?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(PetError::from_status(resp.status().as_u16()))
        }
    }

    pub fn get_log_tail(&self, id: &str, n: usize) -> Result<Vec<String>, PetError> {
        let url = format!("{}/agents/{}/logs", self.base, id);
        let resp = self
            .auth(self.http.get(&url))
            .send()
            .map_err(|_| PetError::Transport)?;
        if !resp.status().is_success() {
            return Err(PetError::from_status(resp.status().as_u16()));
        }
        let body: serde_json::Value = resp.json().map_err(|_| PetError::Decode)?;
        let logs = body.get("logs").and_then(|v| v.as_str()).unwrap_or("");
        Ok(clean_log_tail(logs, n))
    }
}
