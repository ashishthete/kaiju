//! LLM judge for the compare feature: rank candidate diffs via `claude -p`.
//! Candidates are anonymized (A/B/C) so the judge isn't biased by CLI brand.

use std::path::Path;

use kaiju_core::{NexusError, Result};

/// Outcome of running the project's test command in a candidate's worktree.
pub struct TestSummary {
    pub passed: bool,
    /// Short human line (exit status + tail of output) for the judge prompt.
    pub summary: String,
}

/// One anonymized candidate the judge sees.
pub struct Candidate {
    pub label: String,
    /// The real CLI — used by the caller's legend, never put in the judge prompt
    /// (that would defeat the anonymization).
    pub agent_type: String,
    pub diff: String,
    /// Test outcome, when a test command was supplied. `None` = diff-only judging.
    pub test: Option<TestSummary>,
}

/// Per-candidate diff is capped so the prompt stays within argv limits.
const DIFF_CAP_CHARS: usize = 6000;
/// Memory bound on captured test output while draining (rolling tail of bytes).
const OUTPUT_CAP_BYTES: usize = 64 * 1024;
/// How much of the (already-bounded) output is shown to the judge.
const SUMMARY_TAIL_CHARS: usize = 1500;

/// Append `chunk` to `buf`, keeping at most `cap` trailing bytes.
fn cap_tail(buf: &mut Vec<u8>, chunk: &[u8], cap: usize) {
    buf.extend_from_slice(chunk);
    if buf.len() > cap {
        let drop = buf.len() - cap;
        buf.drain(..drop);
    }
}

/// Drain an async reader fully (so the child never blocks on a full pipe),
/// retaining only the last `cap` bytes.
async fn drain_tail<R: tokio::io::AsyncRead + Unpin>(mut r: R, cap: usize) -> Vec<u8> {
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match r.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => cap_tail(&mut buf, &chunk[..n], cap),
        }
    }
    buf
}

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

/// Extract the structured winner label from a verdict ending with `WINNER: X`.
/// Scans from the end and reads the trailing alphabetic label.
pub fn parse_winner(verdict: &str) -> Option<String> {
    verdict.lines().rev().find_map(|line| {
        let rest = line.trim().strip_prefix("WINNER:")?;
        let label: String = rest
            .trim()
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        (!label.is_empty()).then(|| label.to_uppercase())
    })
}

/// Pure: the anonymized judge prompt. Uses only candidate labels (never the CLI
/// name), includes the task, and truncates each diff.
pub fn build_prompt(task: &str, candidates: &[Candidate]) -> String {
    let mut s = String::from(
        "You are judging candidate solutions to a task. Each candidate's work is \
         shown below as either a code diff or, when the task changed no files, the \
         agent's text output — judge whichever is given on its merits. When test \
         results are given, weigh them heavily — a candidate whose tests pass beats \
         one that only looks cleaner. Rank them best to worst on test results, \
         correctness, completeness, and quality. Give a one-line rationale for each \
         candidate, then name the winner. End your reply with a line exactly \
         `WINNER: <letter>`. Be concise.\n\n## Task\n",
    );
    s.push_str(task);
    s.push_str("\n\n## Candidates\n");
    for c in candidates {
        s.push_str(&format!("\n### Candidate {}\n", c.label));
        if let Some(t) = &c.test {
            let verdict = if t.passed { "PASS" } else { "FAIL" };
            s.push_str(&format!("Tests: {verdict} — {}\n", t.summary));
        }
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
        .kill_on_drop(true)
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

/// Run `cmd` (via `sh -c`) in `workdir` with a timeout, capturing pass/fail and
/// a tail of the combined output for the judge. Best-effort: a spawn failure or
/// timeout is reported as a failing test with the reason.
///
/// Both stdout and stderr are drained concurrently so a chatty command never
/// blocks on a full OS pipe. A rolling cap (`OUTPUT_CAP_BYTES`) bounds memory.
/// The child is killed automatically on timeout via `kill_on_drop`.
pub async fn run_tests(workdir: &Path, cmd: &str) -> TestSummary {
    use std::process::Stdio;
    let mut child = match tokio::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(workdir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return TestSummary {
                passed: false,
                summary: format!("could not run tests: {e}"),
            }
        }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    // Drain both pipes (bounded) and reap the exit status. Draining prevents a
    // chatty command from blocking on a full pipe; the rolling cap bounds memory.
    let work = async {
        let (o, e) = tokio::join!(
            async {
                match stdout {
                    Some(s) => drain_tail(s, OUTPUT_CAP_BYTES).await,
                    None => Vec::new(),
                }
            },
            async {
                match stderr {
                    Some(s) => drain_tail(s, OUTPUT_CAP_BYTES).await,
                    None => Vec::new(),
                }
            },
        );
        let status = child.wait().await;
        (status, o, e)
    };
    match tokio::time::timeout(std::time::Duration::from_secs(300), work).await {
        Ok((Ok(status), o, e)) => {
            let mut combined = String::from_utf8_lossy(&o).to_string();
            combined.push_str(&String::from_utf8_lossy(&e));
            let tail: String = {
                let chars: Vec<char> = combined.chars().collect();
                let start = chars.len().saturating_sub(SUMMARY_TAIL_CHARS);
                chars[start..].iter().collect()
            };
            TestSummary {
                passed: status.success(),
                summary: format!("exit {}\n{}", status.code().unwrap_or(-1), tail.trim()),
            }
        }
        Ok((Err(e), _, _)) => TestSummary {
            passed: false,
            summary: format!("could not run tests: {e}"),
        },
        // Timeout: `work` is dropped (releasing its &mut borrow), then `child`
        // drops at end of scope — kill_on_drop sends SIGKILL.
        Err(_) => TestSummary {
            passed: false,
            summary: "tests timed out (300s)".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_tail_keeps_trailing_bytes() {
        let mut buf = Vec::new();
        cap_tail(&mut buf, b"hello", 3);
        assert_eq!(buf, b"llo");
        cap_tail(&mut buf, b"world", 3); // "llo"+"world" -> last 3
        assert_eq!(buf, b"rld");
        // under cap: keep everything
        let mut small = Vec::new();
        cap_tail(&mut small, b"hi", 10);
        assert_eq!(small, b"hi");
    }

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
            Candidate {
                label: "A".into(),
                agent_type: "claude".into(),
                diff: "+ fn a() {}".into(),
                test: None,
            },
            Candidate {
                label: "B".into(),
                agent_type: "codex".into(),
                diff: "+ fn b() {}".into(),
                test: None,
            },
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
        let cands = vec![Candidate {
            label: "A".into(),
            agent_type: "claude".into(),
            diff: big,
            test: None,
        }];
        let p = build_prompt("t", &cands);
        assert!(p.contains("…[truncated]"));
    }

    #[test]
    fn prompt_includes_test_outcome_when_present() {
        let cands = vec![Candidate {
            label: "A".into(),
            agent_type: "claude".into(),
            diff: "+x".into(),
            test: Some(TestSummary {
                passed: true,
                summary: "exit 0 (ok)".into(),
            }),
        }];
        let p = build_prompt("t", &cands);
        assert!(p.contains("Tests: PASS"));
        assert!(p.contains("exit 0 (ok)"));
    }

    #[test]
    fn parse_winner_reads_trailing_marker() {
        assert_eq!(parse_winner("blah\nWINNER: B"), Some("B".to_string()));
        assert_eq!(
            parse_winner("WINNER:  aa  \ntrailing"),
            Some("AA".to_string())
        );
        assert_eq!(parse_winner("no marker here"), None);
    }
}
