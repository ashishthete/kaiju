//! Short-lived, single-use pairing codes. A trusted device asks for a code; a
//! new device redeems it once, within the TTL, to receive its own token. Codes
//! live only in memory — losing them on restart is fine (just re-pair).

use chrono::{DateTime, Duration, Utc};

/// How long a freshly issued code stays redeemable.
pub const CODE_TTL_MINUTES: i64 = 10;

/// A pending, not-yet-redeemed pairing code.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingCode {
    pub code: String,
    pub expires_at: DateTime<Utc>,
}

/// Generate a human-typeable 8-char uppercase code (UUID-derived).
pub fn generate_code() -> String {
    uuid::Uuid::new_v4()
        .to_string()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect::<String>()
        .to_uppercase()
}

/// In-memory set of outstanding codes.
#[derive(Debug, Default)]
pub struct PendingCodes {
    codes: Vec<PendingCode>,
}

impl PendingCodes {
    /// Issue a code valid until `now + CODE_TTL_MINUTES`.
    pub fn issue(&mut self, code: String, now: DateTime<Utc>) {
        self.prune(now);
        self.codes.push(PendingCode {
            code,
            expires_at: now + Duration::minutes(CODE_TTL_MINUTES),
        });
    }

    /// Redeem a code: valid + unexpired removes it and returns true (single
    /// use). Unknown or expired returns false.
    pub fn redeem(&mut self, code: &str, now: DateTime<Utc>) -> bool {
        self.prune(now);
        let before = self.codes.len();
        self.codes.retain(|c| c.code != code);
        self.codes.len() != before
    }

    /// Drop expired codes.
    fn prune(&mut self, now: DateTime<Utc>) {
        self.codes.retain(|c| c.expires_at > now);
    }
}

/// Render a pairing URL as an inline SVG QR code.
pub fn qr_svg(url: &str) -> Result<String, qrcode::types::QrError> {
    use qrcode::render::svg;
    use qrcode::QrCode;
    let code = QrCode::new(url.as_bytes())?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(220, 220)
        .quiet_zone(true)
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn generated_code_is_eight_upper_chars() {
        let c = generate_code();
        assert_eq!(c.len(), 8);
        assert_eq!(c, c.to_uppercase());
    }

    #[test]
    fn redeem_is_single_use() {
        let mut codes = PendingCodes::default();
        let now = at("2026-06-11T00:00:00Z");
        codes.issue("ABC123XY".into(), now);
        assert!(codes.redeem("ABC123XY", now));
        assert!(!codes.redeem("ABC123XY", now)); // already consumed
    }

    #[test]
    fn expired_code_is_rejected() {
        let mut codes = PendingCodes::default();
        codes.issue("ABC123XY".into(), at("2026-06-11T00:00:00Z"));
        // 11 minutes later — past the 10-minute TTL.
        assert!(!codes.redeem("ABC123XY", at("2026-06-11T00:11:00Z")));
    }

    #[test]
    fn unknown_code_is_rejected() {
        let mut codes = PendingCodes::default();
        assert!(!codes.redeem("NOPE0000", at("2026-06-11T00:00:00Z")));
    }

    #[test]
    fn qr_svg_renders_svg_for_url() {
        let svg = qr_svg("http://192.168.1.5:7800/pair?code=ABC123XY").unwrap();
        assert!(svg.contains("<svg"));
    }
}
