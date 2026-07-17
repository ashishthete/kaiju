const MAX_REPLY_BYTES: usize = 4096;

#[derive(Debug, PartialEq, Eq)]
pub enum ReplyError {
    Empty,
    TooLarge,
    ControlChar,
}

/// Trim, then reject empty, oversized, or control-bearing replies.
/// Any Unicode control char (category Cc), including \n \r \t \0, is rejected
/// because the daemon appends its own Enter via tmux send_keys.
pub fn validate_reply(raw: &str) -> Result<String, ReplyError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ReplyError::Empty);
    }
    if trimmed.len() > MAX_REPLY_BYTES {
        return Err(ReplyError::TooLarge);
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(ReplyError::ControlChar);
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_trimmed_single_line() {
        assert_eq!(validate_reply("  yes proceed  ").unwrap(), "yes proceed");
    }

    #[test]
    fn rejects_empty_and_whitespace_only() {
        assert_eq!(validate_reply("").unwrap_err(), ReplyError::Empty);
        assert_eq!(validate_reply("   ").unwrap_err(), ReplyError::Empty);
    }

    #[test]
    fn rejects_newlines_and_control_chars() {
        assert_eq!(validate_reply("a\nb").unwrap_err(), ReplyError::ControlChar);
        assert_eq!(validate_reply("a\rb").unwrap_err(), ReplyError::ControlChar);
        assert_eq!(validate_reply("a\0b").unwrap_err(), ReplyError::ControlChar);
        assert_eq!(validate_reply("a\tb").unwrap_err(), ReplyError::ControlChar);
    }

    #[test]
    fn rejects_over_4kib() {
        let big = "x".repeat(4097);
        assert_eq!(validate_reply(&big).unwrap_err(), ReplyError::TooLarge);
    }
}
