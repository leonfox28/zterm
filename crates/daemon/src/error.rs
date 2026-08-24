//! Stable daemon error categories shared by library, IPC, and CLI projections.

use std::fmt;

use zterm_core::DomainErrorKind;

/// Error with a stable category and bounded user-facing diagnostic.
#[derive(Clone, Eq, PartialEq)]
pub struct DaemonError {
    kind: DomainErrorKind,
    detail: String,
}

impl DaemonError {
    /// Constructs a categorized daemon error.
    #[must_use]
    pub fn new(kind: DomainErrorKind, detail: impl Into<String>) -> Self {
        let mut detail = detail.into();
        if detail.len() > 1024 {
            let mut boundary = 1024;
            while !detail.is_char_boundary(boundary) {
                boundary -= 1;
            }
            detail.truncate(boundary);
        }
        Self { kind, detail }
    }

    /// Stable error category.
    #[must_use]
    pub const fn kind(&self) -> DomainErrorKind {
        self.kind
    }

    /// Bounded diagnostic without secrets or raw SQL.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Debug for DaemonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DaemonError")
            .field("kind", &self.kind)
            .field("detail_len", &self.detail.len())
            .field("detail_present", &!self.detail.is_empty())
            .finish()
    }
}

impl fmt::Display for DaemonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.kind.code(), self.detail)
    }
}

impl std::error::Error for DaemonError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_bounded_detail_without_changing_error_semantics() {
        const DETAIL_SENTINEL: &str = "DAEMON_ERROR_DETAIL_SENTINEL_6f09";

        let unbounded_detail = format!("{DETAIL_SENTINEL}:{}", "界".repeat(400));
        let expected_detail = format!("{DETAIL_SENTINEL}:{}", "界".repeat(330));
        assert_eq!(expected_detail.len(), 1024);

        let error = DaemonError::new(DomainErrorKind::PathUnsafe, unbounded_detail);
        let cloned = error.clone();
        assert_eq!(cloned, error);
        assert_eq!(error.kind(), DomainErrorKind::PathUnsafe);
        assert_eq!(error.detail(), expected_detail);
        assert_eq!(error.to_string(), format!("path_unsafe: {expected_detail}"));

        let debug = format!("{error:?}");
        assert_eq!(
            debug,
            "DaemonError { kind: PathUnsafe, detail_len: 1024, detail_present: true }"
        );
        assert!(!debug.contains(DETAIL_SENTINEL));

        let empty = DaemonError::new(DomainErrorKind::ConfigSyntax, "");
        assert_eq!(
            format!("{empty:?}"),
            "DaemonError { kind: ConfigSyntax, detail_len: 0, detail_present: false }"
        );
    }
}
