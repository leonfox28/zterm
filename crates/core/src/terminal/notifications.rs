//! Validated transient notifications and bounded host-effect retention.

use std::collections::VecDeque;
use std::fmt;

use super::{TerminalClipboardWrite, TerminalHostEffect};

/// Maximum notification OSC body bytes, including command and separators.
pub const MAX_TERMINAL_NOTIFICATION_BYTES: usize = 1_024;
/// Maximum pending notifications at each transient delivery boundary.
pub const MAX_PENDING_TERMINAL_NOTIFICATIONS: usize = 32;

/// Content-free failure to construct an ordinary terminal notification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalNotificationError {
    /// At least one text field must be nonempty.
    Empty,
    /// The canonical OSC body exceeds its byte limit.
    TooLarge,
    /// Notification text contains a control scalar.
    Control,
    /// This OSC 9 value belongs to the reserved numeric subcommand space.
    Subcommand,
    /// An OSC 777 title contains its field delimiter.
    TitleDelimiter,
}

impl fmt::Display for TerminalNotificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "terminal notification is empty",
            Self::TooLarge => "terminal notification exceeds its byte limit",
            Self::Control => "terminal notification contains a control scalar",
            Self::Subcommand => "terminal notification is a reserved OSC subcommand",
            Self::TitleDelimiter => "terminal notification title contains a delimiter",
        })
    }
}

impl std::error::Error for TerminalNotificationError {}

/// Validated OSC 9 message (`title == None`) or OSC 777 title/body.
#[derive(Clone, Eq, PartialEq)]
pub struct TerminalNotification {
    title: Option<String>,
    body: String,
}

impl TerminalNotification {
    /// Constructs an ordinary OSC 9 message, excluding numeric subcommands.
    pub fn osc9(message: String) -> Result<Self, TerminalNotificationError> {
        if message == "4"
            || message.split_once(';').is_some_and(|(prefix, _)| {
                !prefix.is_empty() && prefix.bytes().all(|byte| byte.is_ascii_digit())
            })
        {
            return Err(TerminalNotificationError::Subcommand);
        }
        Self::validated(None, message)
    }

    /// Constructs an OSC 777 notification with an optional empty title or body.
    pub fn osc777(title: String, body: String) -> Result<Self, TerminalNotificationError> {
        if title.contains(';') {
            return Err(TerminalNotificationError::TitleDelimiter);
        }
        Self::validated(Some(title), body)
    }

    fn validated(title: Option<String>, body: String) -> Result<Self, TerminalNotificationError> {
        let title_text = title.as_deref().unwrap_or_default();
        if title_text.is_empty() && body.is_empty() {
            return Err(TerminalNotificationError::Empty);
        }
        let overhead = if title.is_some() {
            "777;notify;;".len()
        } else {
            "9;".len()
        };
        if title_text
            .len()
            .saturating_add(body.len())
            .saturating_add(overhead)
            > MAX_TERMINAL_NOTIFICATION_BYTES
        {
            return Err(TerminalNotificationError::TooLarge);
        }
        if title_text.chars().chain(body.chars()).any(char::is_control) {
            return Err(TerminalNotificationError::Control);
        }
        Ok(Self { title, body })
    }

    /// OSC 777's title (possibly empty), or None for OSC 9.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// OSC 9's complete message or OSC 777's body.
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }
}

impl fmt::Debug for TerminalNotification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalNotification")
            .field("osc", &if self.title.is_some() { 777 } else { 9 })
            .field("title_bytes", &self.title.as_ref().map(String::len))
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

/// Bounded FIFO; on overflow the oldest undelivered request is discarded.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TerminalNotificationQueue(VecDeque<TerminalNotification>);

impl TerminalNotificationQueue {
    /// Admits one distinct request without blocking a producer.
    pub fn push(&mut self, notification: TerminalNotification) {
        if self.0.len() == MAX_PENDING_TERMINAL_NOTIFICATIONS {
            self.0.pop_front();
        }
        self.0.push_back(notification);
    }

    /// Removes the oldest admitted request.
    pub fn pop(&mut self) -> Option<TerminalNotification> {
        self.0.pop_front()
    }

    /// Discards all pending content at an attachment lifetime boundary.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Whether there is no pending request.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Latest clipboard value and independent ordered notification requests.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TerminalHostEffects {
    clipboard: Option<TerminalClipboardWrite>,
    notifications: TerminalNotificationQueue,
}

impl TerminalHostEffects {
    /// Retains an effect according to its own replacement/FIFO policy.
    pub fn push(&mut self, effect: TerminalHostEffect) {
        match effect {
            TerminalHostEffect::ClipboardWrite(write) => self.clipboard = Some(write),
            TerminalHostEffect::Notification(notification) => self.notifications.push(notification),
        }
    }

    /// Takes a pending effect; ordering is guaranteed within notifications only.
    pub fn pop(&mut self) -> Option<TerminalHostEffect> {
        self.clipboard
            .take()
            .map(TerminalHostEffect::ClipboardWrite)
            .or_else(|| {
                self.notifications
                    .pop()
                    .map(TerminalHostEffect::Notification)
            })
    }

    /// Drops all pending effects without retaining a replay record.
    pub fn clear(&mut self) {
        self.clipboard = None;
        self.notifications.clear();
    }

    /// Whether neither effect kind has pending content.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.clipboard.is_none() && self.notifications.is_empty()
    }
}

impl From<TerminalHostEffect> for TerminalHostEffects {
    fn from(effect: TerminalHostEffect) -> Self {
        let mut effects = Self::default();
        effects.push(effect);
        effects
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validated_notification_forms_bounds_and_redaction() {
        let message = TerminalNotification::osc9("结果: secret".into()).expect("valid fixture");
        assert_eq!(message.title(), None);
        assert_eq!(message.body(), "结果: secret");
        assert!(!format!("{message:?}").contains("secret"));
        let notification =
            TerminalNotification::osc777("".into(), "结果;a;b".into()).expect("valid fixture");
        assert_eq!(notification.title(), Some(""));
        for invalid in [
            "", "4", "4;1;30", "99;hello", "a\x1bZ", "a\0b", "a\u{9c}", "a\n",
        ] {
            assert!(TerminalNotification::osc9(invalid.into()).is_err());
        }
        assert!(TerminalNotification::osc9("42".into()).is_ok());
        assert!(TerminalNotification::osc9("x".repeat(1022)).is_ok());
        assert!(TerminalNotification::osc9("x".repeat(1023)).is_err());
        assert!(TerminalNotification::osc777("a;b".into(), "body".into()).is_err());
        assert!(TerminalNotification::osc777("".into(), "".into()).is_err());
        assert!(TerminalNotification::osc777("x".repeat(1011), "x".into()).is_ok());
        assert!(TerminalNotification::osc777("x".repeat(1012), "x".into()).is_err());
    }

    #[test]
    fn effects_keep_latest_clipboard_and_bounded_distinct_notifications() {
        let mut effects = TerminalHostEffects::default();
        for index in 0..40 {
            effects.push(TerminalHostEffect::ClipboardWrite(
                TerminalClipboardWrite::new(index.to_string()).expect("valid fixture"),
            ));
            effects.push(TerminalHostEffect::Notification(
                TerminalNotification::osc9(format!("n{index}")).expect("valid fixture"),
            ));
        }
        assert_eq!(
            effects.pop(),
            Some(TerminalHostEffect::ClipboardWrite(
                TerminalClipboardWrite::new("39".into()).expect("valid fixture")
            ))
        );
        for index in 8..40 {
            let Some(TerminalHostEffect::Notification(value)) = effects.pop() else {
                panic!("notification")
            };
            assert_eq!(value.body(), format!("n{index}"));
        }
        assert!(effects.is_empty());
        for _ in 0..2 {
            effects.push(TerminalHostEffect::Notification(
                TerminalNotification::osc9("same".into()).expect("valid fixture"),
            ));
        }
        assert!(effects.pop().is_some());
        assert!(effects.pop().is_some());
        assert!(effects.pop().is_none());
    }
}
