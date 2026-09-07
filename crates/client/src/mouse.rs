//! Child-mode mouse encoding shared by physical and touch terminal frontends.
use zterm_core::terminal::{ActiveScreen, TerminalModes, TerminalMouseEncoding, TerminalMouseMode};

/// One semantic report, with terminal protocol (one-based) coordinates.
#[derive(Clone, Copy, Debug)]
pub struct MouseInput {
    /// Xterm button/modifier/motion code; wheel up/down are 64/65.
    pub code: u16,
    /// One-based column.
    pub column: u16,
    /// One-based row.
    pub row: u16,
    /// Button release, not wheel motion.
    pub release: bool,
}
impl MouseInput {
    /// Filters reports to the application's declared capture mode.
    pub fn allowed(self, mode: TerminalMouseMode) -> bool {
        if mode == TerminalMouseMode::None {
            return false;
        }
        if self.code & 64 != 0 {
            return true;
        }
        if self.code & 32 != 0 {
            return match mode {
                TerminalMouseMode::ButtonMotion => self.code & 3 != 3,
                TerminalMouseMode::AnyMotion => true,
                _ => false,
            };
        }
        !self.release
            || matches!(
                mode,
                TerminalMouseMode::PressRelease
                    | TerminalMouseMode::ButtonMotion
                    | TerminalMouseMode::AnyMotion
            )
    }
    /// Encodes a report without inventing coordinates outside a legacy encoding.
    pub fn encode(self, encoding: TerminalMouseEncoding) -> Option<Vec<u8>> {
        if self.row == 0 || self.column == 0 {
            return None;
        }
        if encoding == TerminalMouseEncoding::Sgr {
            return Some(
                format!(
                    "\x1b[<{};{};{}{}",
                    self.code,
                    self.column,
                    self.row,
                    if self.release { 'm' } else { 'M' }
                )
                .into_bytes(),
            );
        }
        let code = if self.release {
            (self.code & !3) | 3
        } else {
            self.code
        };
        let mut bytes = b"\x1b[M".to_vec();
        for value in [code, self.column, self.row] {
            let value = value.checked_add(32)?;
            match encoding {
                TerminalMouseEncoding::Default => bytes.push(u8::try_from(value).ok()?),
                TerminalMouseEncoding::Utf8 => bytes.extend_from_slice(
                    char::from_u32(u32::from(value))?
                        .encode_utf8(&mut [0; 4])
                        .as_bytes(),
                ),
                TerminalMouseEncoding::Sgr => unreachable!(),
            }
        }
        Some(bytes)
    }
    /// Routes only declared child mouse / alternate-scroll input.
    pub fn route(self, screen: ActiveScreen, modes: TerminalModes) -> Option<Vec<u8>> {
        if modes.mouse_mode != TerminalMouseMode::None {
            return self
                .allowed(modes.mouse_mode)
                .then(|| self.encode(modes.mouse_encoding))
                .flatten();
        }
        if self.code & 64 != 0 && screen == ActiveScreen::Alternate && modes.alternate_scroll {
            return Some(cursor_wheel(self.code & 1 == 0, modes.application_cursor));
        }
        None
    }
}

/// One wheel step becomes one cursor key only under alternate-scroll ownership.
pub fn cursor_wheel(up: bool, application_cursor: bool) -> Vec<u8> {
    match (up, application_cursor) {
        (true, true) => b"\x1bOA",
        (false, true) => b"\x1bOB",
        (true, false) => b"\x1b[A",
        (false, false) => b"\x1b[B",
    }
    .to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_modes_and_encodings_preserve_click_and_wheel_semantics() {
        let press = MouseInput {
            code: 0,
            column: 7,
            row: 4,
            release: false,
        };
        let release = MouseInput {
            release: true,
            ..press
        };
        assert_eq!(
            press
                .encode(TerminalMouseEncoding::Sgr)
                .expect("valid mouse fixture"),
            b"\x1b[<0;7;4M"
        );
        assert_eq!(
            release
                .encode(TerminalMouseEncoding::Sgr)
                .expect("valid mouse fixture"),
            b"\x1b[<0;7;4m"
        );
        assert_eq!(
            release
                .encode(TerminalMouseEncoding::Default)
                .expect("valid mouse fixture"),
            b"\x1b[M#'$"
        );
        assert!(!press.allowed(TerminalMouseMode::None));
        assert!(press.allowed(TerminalMouseMode::Press));
        assert!(!release.allowed(TerminalMouseMode::Press));
        assert!(release.allowed(TerminalMouseMode::PressRelease));
        let wheel = MouseInput { code: 64, ..press };
        let modes = TerminalModes {
            alternate_scroll: true,
            ..Default::default()
        };
        assert!(wheel.route(ActiveScreen::Main, modes).is_none());
        assert_eq!(
            wheel
                .route(ActiveScreen::Alternate, modes)
                .expect("valid mouse fixture"),
            b"\x1b[A"
        );
        let modes = TerminalModes {
            mouse_mode: TerminalMouseMode::PressRelease,
            mouse_encoding: TerminalMouseEncoding::Sgr,
            ..modes
        };
        assert_eq!(
            wheel
                .route(ActiveScreen::Alternate, modes)
                .expect("valid mouse fixture"),
            b"\x1b[<64;7;4M"
        );
        assert!(
            MouseInput {
                column: 224,
                ..press
            }
            .encode(TerminalMouseEncoding::Default)
            .is_none()
        );
        assert!(
            MouseInput {
                column: 224,
                ..press
            }
            .encode(TerminalMouseEncoding::Utf8)
            .is_some()
        );
        assert!(
            MouseInput { row: 0, ..press }
                .encode(TerminalMouseEncoding::Sgr)
                .is_none()
        );
    }
}
