use serde::Deserialize;
use std::process::Command;

pub struct NexusClient {
    base_url: String,
    http: reqwest::Client,
}

// Only the fields the CLI actually renders are deserialized; any other keys
// in the daemon's JSON response are ignored by serde.
#[derive(Deserialize)]
struct AgentResponse {
    id: String,
    agent_type: String,
    model: Option<String>,
    status: String,
    session_name: String,
    metrics: MetricsResponse,
}

#[derive(Deserialize)]
struct MetricsResponse {
    runtime_secs: u64,
    estimated_cost_usd: Option<f64>,
}

#[derive(Deserialize)]
struct StatusResponse {
    id: String,
    status: String,
    runtime_secs: u64,
    tokens_used: Option<u64>,
    estimated_cost_usd: Option<f64>,
}

#[derive(Deserialize)]
struct LogsResponse {
    logs: String,
}

#[derive(Deserialize)]
struct DiffResponse {
    diff: String,
}

#[derive(Deserialize)]
struct TaskResp {
    id: String,
    status: String,
    agent_type: String,
    prompt: Option<String>,
    agent_id: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: String,
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

impl NexusClient {
    /// Build a client. When `token` is set, it is sent as a bearer header on
    /// every request, so authenticated daemons accept the calls.
    pub fn new(base_url: &str, token: Option<String>) -> Self {
        let http = match token.as_deref().filter(|t| !t.is_empty()) {
            Some(t) => {
                let mut headers = reqwest::header::HeaderMap::new();
                if let Ok(value) = reqwest::header::HeaderValue::from_str(&format!("Bearer {t}")) {
                    headers.insert(reqwest::header::AUTHORIZATION, value);
                }
                reqwest::Client::builder()
                    .default_headers(headers)
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new())
            }
            None => reqwest::Client::new(),
        };
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http,
        }
    }

    pub async fn start(
        &self,
        agent_type: &str,
        workspace: &str,
        model: Option<String>,
        prompt: Option<String>,
        isolate: bool,
        batch: bool,
    ) -> Result<()> {
        let resp = self
            .http
            .post(format!("{}/agents", self.base_url))
            .json(&serde_json::json!({
                "agent_type": agent_type,
                "workspace": workspace,
                "model": model,
                "prompt": prompt,
                "auto_start": true,
                "isolate": isolate,
                "batch": batch,
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let err: ErrorResponse = resp.json().await?;
            return Err(err.error.into());
        }

        let agent: AgentResponse = resp.json().await?;
        println!("Agent started:");
        println!("  ID:      {}", agent.id);
        println!("  Type:    {}", agent.agent_type);
        println!("  Session: {}", agent.session_name);
        println!("  Status:  {}", agent.status);
        println!();
        println!("Attach with: kaiju attach {}", agent.id);

        Ok(())
    }

    pub async fn list(&self, active_only: bool) -> Result<()> {
        let resp = self
            .http
            .get(format!("{}/agents", self.base_url))
            .send()
            .await?;

        let agents: Vec<AgentResponse> = resp.json().await?;

        let filtered: Vec<&AgentResponse> = if active_only {
            agents
                .iter()
                .filter(|a| {
                    matches!(
                        a.status.as_str(),
                        "starting" | "running" | "waitingforinput" | "stuck"
                    )
                })
                .collect()
        } else {
            agents.iter().collect()
        };

        if filtered.is_empty() {
            println!("No agents found.");
            return Ok(());
        }

        println!(
            "{:<12} {:<10} {:<10} {:<18} {:<10} {:>8}",
            "ID", "TYPE", "MODEL", "STATUS", "RUNTIME", "COST"
        );
        println!("{}", "-".repeat(72));

        for a in filtered {
            let short_id = if a.id.len() > 10 { &a.id[..10] } else { &a.id };
            let model = a.model.as_deref().unwrap_or("-");
            let cost = a
                .metrics
                .estimated_cost_usd
                .map(|c| format!("${:.2}", c))
                .unwrap_or_else(|| "-".to_string());
            let runtime = format_duration(a.metrics.runtime_secs);

            println!(
                "{:<12} {:<10} {:<10} {:<18} {:<10} {:>8}",
                short_id, a.agent_type, model, a.status, runtime, cost
            );
        }

        Ok(())
    }

    pub async fn status(&self, id: &str) -> Result<()> {
        let resp = self
            .http
            .get(format!("{}/agents/{id}/status", self.base_url))
            .send()
            .await?;

        if !resp.status().is_success() {
            let err: ErrorResponse = resp.json().await?;
            return Err(err.error.into());
        }

        let status: StatusResponse = resp.json().await?;
        println!("Agent:   {}", status.id);
        println!("Status:  {}", status.status);
        println!("Runtime: {}", format_duration(status.runtime_secs));
        if let Some(tokens) = status.tokens_used {
            println!("Tokens:  {tokens}");
        }
        if let Some(cost) = status.estimated_cost_usd {
            println!("Cost:    ${cost:.2}");
        }

        Ok(())
    }

    pub async fn logs(&self, id: &str) -> Result<()> {
        let resp = self
            .http
            .get(format!("{}/agents/{id}/logs", self.base_url))
            .send()
            .await?;

        if !resp.status().is_success() {
            let err: ErrorResponse = resp.json().await?;
            return Err(err.error.into());
        }

        let logs: LogsResponse = resp.json().await?;
        print!("{}", logs.logs);

        Ok(())
    }

    pub async fn diff(&self, id: &str) -> Result<()> {
        let resp = self
            .http
            .get(format!("{}/agents/{id}/diff", self.base_url))
            .send()
            .await?;

        if !resp.status().is_success() {
            let err: ErrorResponse = resp.json().await?;
            return Err(err.error.into());
        }

        let diff: DiffResponse = resp.json().await?;
        if diff.diff.trim().is_empty() {
            println!("No changes.");
        } else {
            print!("{}", diff.diff);
        }

        Ok(())
    }

    pub async fn stop(&self, id: &str) -> Result<()> {
        let resp = self
            .http
            .post(format!("{}/agents/{id}/stop", self.base_url))
            .send()
            .await?;

        if !resp.status().is_success() {
            let err: ErrorResponse = resp.json().await?;
            return Err(err.error.into());
        }

        let agent: AgentResponse = resp.json().await?;
        println!("Agent {} stopped.", agent.id);

        Ok(())
    }

    pub async fn resume(&self, id: &str) -> Result<()> {
        let resp = self
            .http
            .post(format!("{}/agents/{id}/resume", self.base_url))
            .send()
            .await?;

        if !resp.status().is_success() {
            let err: ErrorResponse = resp.json().await?;
            return Err(err.error.into());
        }

        let agent: AgentResponse = resp.json().await?;
        println!("Agent {} resumed.", agent.id);

        Ok(())
    }

    pub async fn send(&self, id: &str, message: &str) -> Result<()> {
        let resp = self
            .http
            .post(format!("{}/agents/{id}/input", self.base_url))
            .json(&serde_json::json!({ "text": message }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let err: ErrorResponse = resp.json().await?;
            return Err(err.error.into());
        }

        println!("Sent to agent {id}.");

        Ok(())
    }

    pub async fn interrupt(&self, id: &str) -> Result<()> {
        let resp = self
            .http
            .post(format!("{}/agents/{id}/interrupt", self.base_url))
            .send()
            .await?;

        if !resp.status().is_success() {
            let err: ErrorResponse = resp.json().await?;
            return Err(err.error.into());
        }

        println!("Interrupt sent to agent {id}.");

        Ok(())
    }

    pub async fn remove(&self, id: &str) -> Result<()> {
        let resp = self
            .http
            .delete(format!("{}/agents/{id}", self.base_url))
            .send()
            .await?;

        if !resp.status().is_success() {
            let err: ErrorResponse = resp.json().await?;
            return Err(err.error.into());
        }

        println!("Agent {id} removed.");

        Ok(())
    }

    pub async fn attach(&self, id: &str) -> Result<()> {
        // Get the session name from the API
        let resp = self
            .http
            .get(format!("{}/agents/{id}", self.base_url))
            .send()
            .await?;

        if !resp.status().is_success() {
            let err: ErrorResponse = resp.json().await?;
            return Err(err.error.into());
        }

        let agent: AgentResponse = resp.json().await?;

        // Exec into tmux attach (replaces this process)
        let status = Command::new("tmux")
            .args(["attach-session", "-t", &agent.session_name])
            .status()?;

        if !status.success() {
            return Err(format!("tmux attach failed for session {}", agent.session_name).into());
        }

        Ok(())
    }

    pub async fn submit(
        &self,
        agent_type: &str,
        workspace: &str,
        model: Option<String>,
        prompt: Option<String>,
        isolate: bool,
    ) -> Result<()> {
        let resp = self
            .http
            .post(format!("{}/tasks", self.base_url))
            .json(&serde_json::json!({
                "agent_type": agent_type,
                "workspace": workspace,
                "model": model,
                "prompt": prompt,
                "isolate": isolate,
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let err: ErrorResponse = resp.json().await?;
            return Err(err.error.into());
        }

        let task: TaskResp = resp.json().await?;
        println!("Task queued:");
        println!("  ID:     {}", task.id);
        println!("  Status: {}", task.status);
        Ok(())
    }

    pub async fn queue(&self) -> Result<()> {
        let resp = self
            .http
            .get(format!("{}/tasks", self.base_url))
            .send()
            .await?;

        let tasks: Vec<TaskResp> = resp.json().await?;
        if tasks.is_empty() {
            println!("No tasks.");
            return Ok(());
        }

        println!(
            "{:<12} {:<10} {:<10} {:<40}",
            "ID", "TYPE", "STATUS", "DETAIL"
        );
        println!("{}", "-".repeat(72));
        for t in &tasks {
            let id = if t.id.len() > 10 { &t.id[..10] } else { &t.id };
            let detail = t
                .error
                .clone()
                .or_else(|| t.agent_id.clone())
                .or_else(|| t.prompt.clone())
                .unwrap_or_else(|| "-".to_string());
            println!(
                "{:<12} {:<10} {:<10} {:<40}",
                id, t.agent_type, t.status, detail
            );
        }
        Ok(())
    }

    pub async fn cancel_task(&self, id: &str) -> Result<()> {
        let resp = self
            .http
            .post(format!("{}/tasks/{id}/cancel", self.base_url))
            .send()
            .await?;

        if !resp.status().is_success() {
            let err: ErrorResponse = resp.json().await?;
            return Err(err.error.into());
        }

        println!("Task {id} canceled.");
        Ok(())
    }
}

fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}
