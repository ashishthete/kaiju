/// Strip ANSI/control sequences from a captured tmux pane, then return the
/// last `n` non-empty lines as plain text. Never renders as rich text.
pub fn clean_log_tail(raw: &str, n: usize) -> Vec<String> {
    let stripped = strip_ansi_escapes::strip(raw.as_bytes());
    let text = String::from_utf8_lossy(&stripped);
    let lines: Vec<String> = text
        .lines()
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.trim().is_empty())
        .collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_ansi_and_tails_last_n() {
        let raw = "\x1b[32mstart\x1b[0m\nline two\n\nline three\nline four\n";
        let out = clean_log_tail(raw, 2);
        assert_eq!(out, vec!["line three".to_string(), "line four".to_string()]);
    }

    #[test]
    fn empty_input_yields_no_lines() {
        assert!(clean_log_tail("", 5).is_empty());
    }

    #[test]
    fn fewer_lines_than_n_returns_all() {
        let out = clean_log_tail("only one\n", 5);
        assert_eq!(out, vec!["only one".to_string()]);
    }
}
