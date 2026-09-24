//! Validated shared web-link values; no platform opening or terminal parsing.
use std::fmt;

/// Maximum distinct links retained by a terminal or one semantic message.
pub const MAX_TERMINAL_HYPERLINKS: usize = 1_024;
/// Maximum total retained URI/id bytes.
pub const MAX_TERMINAL_HYPERLINK_BYTES: usize = 1024 * 1024;
/// Maximum canonical target URI length.
pub const MAX_TERMINAL_HYPERLINK_URI_BYTES: usize = 1_024;
/// Maximum explicit or generated link identity length.
pub const MAX_TERMINAL_HYPERLINK_ID_BYTES: usize = 128;

/// An invalid, unsupported or oversized hyperlink, without its payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidTerminalHyperlink;

impl fmt::Display for InvalidTerminalHyperlink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid terminal web hyperlink")
    }
}
impl std::error::Error for InvalidTerminalHyperlink {}

/// Immutable, bounded HTTP(S) target shared by all cells in a link span.
#[derive(Clone, Eq, PartialEq, Hash)]
pub struct TerminalHyperlink {
    id: Box<str>,
    uri: Box<str>,
}

impl TerminalHyperlink {
    /// Validates the OSC identity and canonicalizes an absolute web target.
    pub fn new(id: &str, uri: &str) -> Result<Self, InvalidTerminalHyperlink> {
        if id.len() > MAX_TERMINAL_HYPERLINK_ID_BYTES
            || id.chars().any(|c| c.is_control() || matches!(c, ':' | ';'))
            || uri.len() > MAX_TERMINAL_HYPERLINK_URI_BYTES
            || uri.chars().any(char::is_control)
        {
            return Err(InvalidTerminalHyperlink);
        }
        let parsed = url::Url::parse(uri).map_err(|_| InvalidTerminalHyperlink)?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            return Err(InvalidTerminalHyperlink);
        }
        let canonical = parsed.to_string();
        if canonical.len() > MAX_TERMINAL_HYPERLINK_URI_BYTES {
            return Err(InvalidTerminalHyperlink);
        }
        Ok(Self {
            id: id.into(),
            uri: canonical.into_boxed_str(),
        })
    }
    /// Link identity, distinct from the displayed text.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Canonical HTTP(S) target. Open only after an explicit user action.
    #[must_use]
    pub fn uri(&self) -> &str {
        &self.uri
    }
    /// URI/id payload bytes for message and host quotas.
    #[must_use]
    pub fn payload_bytes(&self) -> usize {
        self.id.len() + self.uri.len()
    }
    /// Shared allocation accounting, including the Arc's reference counts.
    #[must_use]
    pub fn allocation_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + 2 * std::mem::size_of::<usize>() + self.payload_bytes()
    }
}
impl fmt::Debug for TerminalHyperlink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TerminalHyperlink([REDACTED])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn targets_are_canonical_bounded_web_urls_and_debug_is_redacted() {
        let link = TerminalHyperlink::new("label", "HTTPS://Example.COM/秘密").expect("web");
        assert_eq!(link.uri(), "https://example.com/%E7%A7%98%E5%AF%86");
        assert!(!format!("{link:?}").contains("example"));
        for uri in [
            "file:///remote",
            "javascript:alert(1)",
            "https://",
            "https://host/\nsecret",
            "relative/path",
        ] {
            assert!(TerminalHyperlink::new("id", uri).is_err());
        }
        assert!(TerminalHyperlink::new("a;bad", "https://example.com").is_err());
        assert!(TerminalHyperlink::new(&"x".repeat(129), "https://example.com").is_err());
    }
}
