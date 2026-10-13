//! Shell quoting for values interpolated into an agent's launch command.
//!
//! Launch commands are run with `sh -c`, so a workspace path with a space (or a
//! model name containing shell syntax) must be quoted to stay a single word.

/// Quote `value` as one shell word. Plain values are returned unchanged so the
/// common case stays readable in logs; anything else is single-quoted.
pub(crate) fn quote(value: &str) -> String {
    let is_plain = !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-./:=@+,".contains(c));
    if is_plain {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_values_are_left_alone() {
        assert_eq!(quote("/home/me/repo"), "/home/me/repo");
        assert_eq!(quote("claude-opus-4-8"), "claude-opus-4-8");
    }

    #[test]
    fn spaces_and_shell_syntax_are_single_quoted() {
        assert_eq!(quote("/Users/me/My Projects"), "'/Users/me/My Projects'");
        assert_eq!(quote("x; rm -rf ~"), "'x; rm -rf ~'");
        assert_eq!(quote("$(whoami)"), "'$(whoami)'");
        assert_eq!(quote(""), "''");
    }

    #[test]
    fn embedded_single_quote_is_escaped() {
        assert_eq!(quote("it's"), "'it'\\''s'");
    }
}
