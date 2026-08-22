//! Stable daemon error categories shared by library, IPC, and CLI projections.

use std::fmt;

use zterm_core::DomainErrorKind;

/// Error with a stable category and bounded user-facing diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
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
    fn long_utf8_diagnostics_are_bounded_without_splitting_a_character() {
        let error = DaemonError::new(DomainErrorKind::ConfigSyntax, "界".repeat(400));

        assert!(error.detail().len() <= 1024);
        assert!(error.detail().chars().all(|character| character == '界'));
    }
}
