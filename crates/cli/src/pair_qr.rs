//! Local presentation of one canonical bearer ticket; no pairing state owner.
use crate::CliError;
use qrcode::{QrCode, render::unicode::Dense1x2};
use std::io::{self, IsTerminal};
use zeroize::Zeroizing;

const QUIET: usize = 4;

/// None denotes a pipe; an unavailable terminal width falls back to manual text.
pub(crate) fn stdout_width() -> Option<Result<usize, CliError>> {
    if !io::stdout().is_terminal() {
        return None;
    }
    #[cfg(unix)]
    {
        Some(
            rustix::termios::tcgetwinsize(io::stdout())
                .map(|size| usize::from(size.ws_col))
                .map_err(|error| CliError::Io(format!("read QR terminal width: {error}"))),
        )
    }
    #[cfg(not(unix))]
    {
        Some(Err(CliError::Usage("terminal width is unavailable".into())))
    }
}

pub(crate) fn present(
    ticket: &str,
    width: Option<Result<usize, CliError>>,
) -> (Zeroizing<String>, Option<CliError>) {
    let mut output = Zeroizing::new(String::new());
    let mut fallback = None;
    if let Some(width) = width {
        match width.and_then(|columns| terminal(ticket, columns)) {
            Ok(qr) => {
                output.push_str(&qr);
                output.push('\n');
            }
            Err(error) => fallback = Some(error),
        }
    }
    // Every presentation uses the same already-created ticket, exactly once as text.
    output.push_str(ticket);
    output.push('\n');
    (output, fallback)
}

fn terminal(ticket: &str, columns: usize) -> Result<Zeroizing<String>, CliError> {
    let code = QrCode::new(ticket.as_bytes())
        .map_err(|_| CliError::Usage("ticket exceeds QR capacity".into()))?;
    if columns <= code.width() + 2 * QUIET {
        return Err(CliError::Usage("terminal is too narrow for this QR".into()));
    }
    let raster = Zeroizing::new(code.render::<Dense1x2>().quiet_zone(true).build());
    let mut output = Zeroizing::new(String::new());
    for line in raster.lines() {
        // Fixed monochrome colors, including the quiet zone, in either theme.
        output.push_str("\x1b[30;47m");
        output.push_str(line);
        output.push_str("\x1b[0m\n");
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_presentation_preserves_one_manual_ticket_in_all_modes() {
        let ticket = "zterm-pair-v1:synthetic-test-content";
        let (pipe, fallback) = present(ticket, None);
        assert_eq!(*pipe, format!("{ticket}\n"));
        assert!(fallback.is_none());

        let (wide, fallback) = present(ticket, Some(Ok(200)));
        assert!(fallback.is_none());
        let qr = terminal(ticket, 200).expect("synthetic QR");
        assert_eq!(*wide, format!("{}\n{ticket}\n", qr.as_str()));
        assert!(wide.starts_with("\x1b[30;47m"));
        assert_eq!(wide.matches(ticket).count(), 1);

        for width in [
            Some(Ok(10)),
            Some(Err(CliError::Io("width unavailable".into()))),
        ] {
            let (manual, fallback) = present(ticket, width);
            assert_eq!(*manual, *pipe);
            assert!(!fallback.expect("fallback").to_string().contains(ticket));
        }
        let oversized = "x".repeat(zterm_core::MAX_TICKET_TEXT_BYTES);
        let (manual, fallback) = present(&oversized, Some(Ok(200)));
        assert_eq!(*manual, format!("{oversized}\n"));
        let error = fallback.expect("capacity fallback");
        assert!(!format!("{error} {error:?}").contains(&oversized));
    }
}
