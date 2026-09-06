//! Bounded, transport-neutral terminal color observations.
use crate::Revision;

/// Number of indexed and special color slots.
pub const TERMINAL_COLOR_SLOTS: usize = 262;
/// Default text foreground slot.
pub const COLOR_FOREGROUND: usize = 256;
/// Default text background slot.
pub const COLOR_BACKGROUND: usize = 257;
/// Cursor block slot.
pub const COLOR_CURSOR: usize = 258;
/// Cursor glyph slot.
pub const COLOR_CURSOR_TEXT: usize = 259;
/// Selection foreground slot.
pub const COLOR_SELECTION_FOREGROUND: usize = 260;
/// Selection background slot.
pub const COLOR_SELECTION_BACKGROUND: usize = 261;

/// An observation is distinct from an absent application override.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TerminalColorValue {
    /// No fixed value was observed.
    #[default]
    Unknown,
    /// A concrete eight-bit RGB color.
    Rgb(u8, u8, u8),
    /// A special role derives its color from the displayed cell.
    Dynamic,
}

/// Terminal-reported appearance preference, independent of rendition.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TerminalAppearance {
    /// The controller could not observe the preference.
    #[default]
    Unknown,
    /// Dark appearance.
    Dark,
    /// Light appearance.
    Light,
}

/// Complete bounded observations supplied by one controlling frontend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalColorProfile {
    /// Indexed colors followed by the six special roles.
    pub values: Box<[TerminalColorValue; TERMINAL_COLOR_SLOTS]>,
    /// Reported appearance preference.
    pub appearance: TerminalAppearance,
}
impl Default for TerminalColorProfile {
    fn default() -> Self {
        Self {
            values: Box::new([TerminalColorValue::Unknown; TERMINAL_COLOR_SLOTS]),
            appearance: TerminalAppearance::Unknown,
        }
    }
}
impl TerminalColorProfile {
    /// Rejects dynamic values for roles without dynamic rendering semantics.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        !self.values[..COLOR_CURSOR].contains(&TerminalColorValue::Dynamic)
    }
}

/// Effective colors attached to a semantic screen or history revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalColorSnapshot {
    /// Effective mapped colors, not mutable overrides or controller identity.
    pub profile: TerminalColorProfile,
    /// Revision at which effective state last changed.
    pub changed_at: Revision,
    /// Screen reverse mapping, also used for unobserved inheritance sources.
    pub reverse: bool,
    /// Explicit cursor customization requires software presentation.
    pub custom_cursor: bool,
    /// Physical source of each unknown observation, retained across color stacks.
    pub inherited_sources: Box<[u16; TERMINAL_COLOR_SLOTS]>,
}
impl Default for TerminalColorSnapshot {
    fn default() -> Self {
        Self {
            profile: TerminalColorProfile::default(),
            changed_at: Revision::default(),
            reverse: false,
            custom_cursor: false,
            inherited_sources: Box::new(std::array::from_fn(|slot| {
                u16::try_from(slot).expect("bounded color slot")
            })),
        }
    }
}
impl TerminalColorSnapshot {
    /// Validates color roles and the enclosing revision.
    #[must_use]
    pub fn is_valid_at(&self, revision: Revision) -> bool {
        self.profile.is_valid()
            && self.changed_at <= revision
            && self
                .inherited_sources
                .iter()
                .all(|source| usize::from(*source) < TERMINAL_COLOR_SLOTS)
    }
    /// Maps a logical slot to its inherited source under screen reversal.
    #[must_use]
    pub const fn source(&self, slot: usize) -> usize {
        self.inherited_sources[slot] as usize
    }
}

/// Semantic underline shape, independent of any font renderer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TerminalUnderline {
    /// No underline.
    #[default]
    None,
    /// Single line.
    Single,
    /// Double line.
    Double,
    /// Curly line.
    Curly,
    /// Dotted line.
    Dotted,
    /// Dashed line.
    Dashed,
}

/// Parses the numeric X11 RGB forms used by terminal color observations.
/// Named-color lookup belongs to the terminal protocol owner.
#[must_use]
pub fn parse_terminal_rgb(value: &str) -> Option<TerminalColorValue> {
    let value = match value.split_once('@') {
        Some((value, alpha)) if alpha.parse::<f64>().ok()? == 1.0 => value,
        Some(_) => return None,
        None => value,
    };
    let rgb: [u8; 3] = if let Some(s) = value.strip_prefix("rgb:") {
        let mut parts = s.split('/');
        let mut channels = [0; 3];
        for channel in &mut channels {
            let p = parts.next()?;
            if p.is_empty() || p.len() > 4 {
                return None;
            }
            *channel = u8::try_from(
                u32::from_str_radix(p, 16).ok()? * 255 / ((1u32 << (p.len() * 4)) - 1),
            )
            .ok()?;
        }
        if parts.next().is_some() {
            return None;
        }
        channels
    } else if let Some(s) = value.strip_prefix('#') {
        if !matches!(s.len(), 3 | 6 | 9 | 12) || !s.is_ascii() {
            return None;
        }
        let width = s.len() / 3;
        let mut channels = [0; 3];
        for (i, channel) in channels.iter_mut().enumerate() {
            let v = u32::from_str_radix(&s[i * width..(i + 1) * width], 16).ok()?;
            *channel = u8::try_from((v << (16 - width * 4)) >> 8).ok()?;
        }
        channels
    } else {
        let s = value.strip_prefix("rgbi:")?;
        let mut parts = s.split('/');
        let mut channels = [0; 3];
        for channel in &mut channels {
            let p = parts.next()?;
            if p.is_empty()
                || !p
                    .bytes()
                    .all(|b| b.is_ascii_digit() || b"+-.eE".contains(&b))
            {
                return None;
            }
            let v = p.parse::<f64>().ok()?;
            if !v.is_finite() {
                return None;
            }
            *channel = (v.clamp(0.0, 1.0) * 255.0) as u8;
        }
        if parts.next().is_some() {
            return None;
        }
        channels
    };
    Some(TerminalColorValue::Rgb(rgb[0], rgb[1], rgb[2]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_colors_preserve_x11_precision_and_reject_invalid_or_translucent_values() {
        for (text, rgb) in [
            ("#3a7", (48, 160, 112)),
            ("rgb:3/a/7", (51, 170, 119)),
            ("#123456789", (18, 69, 120)),
            ("rgbi:-1/.5/2", (0, 127, 255)),
            ("rgb:ff/00/aa@1.0", (255, 0, 170)),
        ] {
            assert_eq!(
                parse_terminal_rgb(text),
                Some(TerminalColorValue::Rgb(rgb.0, rgb.1, rgb.2)),
                "{text}"
            );
        }
        for text in [
            "#abcd",
            "rgb:fffff/0/0",
            "rgb:ff/ff/",
            "rgbi:NaN/0/0",
            "rgbi:inf/0/0",
            "rgbi:1e309/0/0",
            "#ffffff@.5",
        ] {
            assert_eq!(parse_terminal_rgb(text), None, "{text}");
        }
    }
    #[test]
    fn color_snapshot_rejects_invalid_dynamic_roles_sources_and_future_stamps() {
        let mut colors = TerminalColorSnapshot::default();
        colors.profile.values[COLOR_FOREGROUND] = TerminalColorValue::Dynamic;
        assert!(!colors.is_valid_at(Revision::new(1)));
        colors.profile.values[COLOR_FOREGROUND] = TerminalColorValue::Unknown;
        colors.inherited_sources[0] = 262;
        assert!(!colors.is_valid_at(Revision::new(1)));
        colors.inherited_sources[0] = 0;
        colors.changed_at = Revision::new(2);
        assert!(!colors.is_valid_at(Revision::new(1)));
    }
}
