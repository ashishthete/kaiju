//! LLM judge for the compare feature: rank candidate diffs via `claude -p`.
//! Candidates are anonymized (A/B/C) so the judge isn't biased by CLI brand.

use std::path::Path;

use kaiju_core::{NexusError, Result};

/// One anonymized candidate the judge sees.
pub struct Candidate {
    pub label: String,
    /// The real CLI — used by the caller's legend, never put in the judge prompt
    /// (that would defeat the anonymization).
    pub agent_type: String,
    pub diff: String,
}

/// Per-candidate diff is capped so the prompt stays within argv limits.
const DIFF_CAP_CHARS: usize = 6000;

/// 0 -> "A", 1 -> "B", ... 25 -> "Z", 26 -> "AA".
pub fn label_for(mut index: usize) -> String {
    let mut s = String::new();
    loop {
        s.insert(0, (b'A' + (index % 26) as u8) as char);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    s
}

/// Pure: the anonymized judge prompt. Uses only candidate labels (never the CLI
/// name), includes the task, and truncates each diff.
pub fn build_prompt(task: &str, candidates: &[Candidate]) -> String {
    let mut s = String::from(
        "You are judging candidate solutions to a coding task. Rank them best to \
         worst on correctness, completeness, and code quality. Give a one-line \
         rationale for each candidate, then name the winner. Be concise.\n\n## Task\n",
    );
    s.push_str(task);
    s.push_str("\n\n## Candidates\n");
    for c in candidates {
        s.push_str(&format!("\n### Candidate {}\n", c.label));
        let truncated: String = c.diff.chars().take(DIFF_CAP_CHARS).collect();
        s.push_str(&truncated);
        if c.diff.chars().count() > DIFF_CAP_CHARS {
            s.push_str("\n…[truncated]");
        }
        s.push('\n');
    }
    s
}

/// Run the judge: `claude -p <prompt>` in `workspace`, with a timeout. Returns
/// the model's stdout (the verdict). Side effect; not unit-tested.
pub async fn run_judge(workspace: &Path, prompt: &str) -> Result<String> {
    let bin = std::env::var("KAIJU_CLAUDE_BIN")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "claude".to_string());
    let fut = tokio::process::Command::new(&bin)
        .arg("-p")
        .arg(prompt)
        .current_dir(workspace)
        .output();
    let out = tokio::time::timeout(std::time::Duration::from_secs(180), fut)
        .await
        .map_err(|_| NexusError::Adapter("judge timed out".to_string()))?
        .map_err(|e| NexusError::Adapter(format!("judge failed to start ({bin}): {e}")))?;
    if !out.status.success() {
        return Err(NexusError::Adapter(format!(
            "judge exited with error: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        return Err(NexusError::Adapter("judge returned no output".to_string()));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_sequential() {
        assert_eq!(label_for(0), "A");
        assert_eq!(label_for(1), "B");
        assert_eq!(label_for(25), "Z");
        assert_eq!(label_for(26), "AA");
    }

    #[test]
    fn prompt_is_anonymized_and_has_task() {
        let cands = vec![
            Candidate { label: "A".into(), agent_type: "claude".into(), diff: "+ fn a() {}".into() },
            Candidate { label: "B".into(), agent_type: "codex".into(), diff: "+ fn b() {}".into() },
        ];
        let p = build_prompt("add a function", &cands);
        assert!(p.contains("add a function"));
        assert!(p.contains("### Candidate A"));
        assert!(p.contains("### Candidate B"));
        assert!(!p.contains("claude"));
        assert!(!p.contains("codex"));
    }

    #[test]
    fn long_diff_is_truncated() {
        let big = "x".repeat(DIFF_CAP_CHARS + 500);
        let cands = vec![Candidate { label: "A".into(), agent_type: "claude".into(), diff: big }];
        let p = build_prompt("t", &cands);
        assert!(p.contains("…[truncated]"));
    }
}
