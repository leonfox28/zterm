//! Local presentation of one canonical bearer ticket; no pairing state owner.
use crate::CliError;
use qrcode::{Color, QrCode, render::unicode::Dense1x2};
use std::{io, path::Path};
use zeroize::Zeroizing;

const QUIET: usize = 4;
const SCALE: usize = 8;

pub(crate) fn check_destination(path: &Path) -> Result<(), CliError> {
    match path.symlink_metadata() {
        Ok(_) => Err(CliError::Usage(
            "QR destination already exists; choose a new file".into(),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CliError::Io(format!("check QR destination: {error}"))),
    }
}
fn encode(ticket: &str) -> Result<QrCode, CliError> {
    QrCode::new(ticket.as_bytes()).map_err(|_| CliError::Usage("ticket exceeds QR capacity".into()))
}

pub(crate) fn terminal(ticket: &str) -> Result<Zeroizing<String>, CliError> {
    let code = encode(ticket)?;
    #[cfg(unix)]
    {
        let size = rustix::termios::tcgetwinsize(io::stdout())
            .map_err(|error| CliError::Io(format!("read QR terminal width: {error}")))?;
        if usize::from(size.ws_col) <= code.width() + 2 * QUIET {
            return Err(CliError::Usage(
                "terminal is too narrow for this QR; use --qr-image".into(),
            ));
        }
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

pub(crate) fn write_png(ticket: &str, path: &Path) -> Result<(), CliError> {
    let code = encode(ticket)?;
    let side = (code.width() + QUIET * 2) * SCALE;
    let mut pixels = Zeroizing::new(vec![255; side * side]);
    for y in 0..code.width() {
        for x in 0..code.width() {
            if code[(x, y)] == Color::Dark {
                for dy in 0..SCALE {
                    let start = ((y + QUIET) * SCALE + dy) * side + (x + QUIET) * SCALE;
                    pixels[start..start + SCALE].fill(0);
                }
            }
        }
    }
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| CliError::Io(format!("create private QR file: {error}")))?;
    {
        let side = u32::try_from(side).expect("bounded QR dimension fits PNG");
        let mut encoder = png::Encoder::new(temporary.as_file_mut(), side, side);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|_| CliError::Io("write QR PNG header".into()))?;
        writer
            .write_image_data(&pixels)
            .map_err(|_| CliError::Io("write QR PNG pixels".into()))?;
        writer
            .finish()
            .map_err(|_| CliError::Io("finish QR PNG".into()))?;
    }
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| CliError::Io(format!("sync QR file: {error}")))?;
    temporary
        .persist_noclobber(path)
        .map_err(|error| CliError::Io(format!("save new QR file: {}", error.error)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn qr_flags_are_explicit_and_exclusive() {
        assert!(
            crate::Cli::try_parse_from(["zterm", "pair", "create", "--qr", "--qr-image", "qr.png"])
                .is_err()
        );
        for flags in [vec![], vec!["--qr"], vec!["--qr-image", "qr.png"]] {
            assert!(
                crate::Cli::try_parse_from(["zterm", "pair", "create"].into_iter().chain(flags))
                    .is_ok()
            );
        }
    }

    #[test]
    fn png_preserves_modules_quiet_zone_and_existing_file() {
        let directory = tempfile::tempdir().expect("valid test fixture");
        let path = directory.path().join("ticket.png");
        let ticket = "zterm-pair-v1:synthetic-test-content";
        write_png(ticket, &path).expect("valid test fixture");
        let original = std::fs::read(&path).expect("valid test fixture");
        assert!(check_destination(&path).is_err());
        assert!(write_png("different-ticket", &path).is_err());
        assert_eq!(original, std::fs::read(&path).expect("valid test fixture"));
        let mut reader = png::Decoder::new(io::Cursor::new(original))
            .read_info()
            .expect("valid test fixture");
        let mut pixels = vec![0; reader.output_buffer_size().expect("valid test fixture")];
        let info = reader.next_frame(&mut pixels).expect("valid test fixture");
        let code = encode(ticket).expect("valid test fixture");
        let modules = code.width() + QUIET * 2;
        assert_eq!(info.width as usize, modules * SCALE);
        assert_eq!(info.width, info.height);
        for y in 0..modules * SCALE {
            for x in 0..modules * SCALE {
                let (mx, my) = (x / SCALE, y / SCALE);
                let dark = (QUIET..modules - QUIET).contains(&mx)
                    && (QUIET..modules - QUIET).contains(&my)
                    && code[(mx - QUIET, my - QUIET)] == Color::Dark;
                assert_eq!(pixels[y * modules * SCALE + x], if dark { 0 } else { 255 });
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path)
                    .expect("valid test fixture")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn oversized_ticket_does_not_create_partial_image() {
        let directory = tempfile::tempdir().expect("valid test fixture");
        let path = directory.path().join("ticket.png");
        assert!(write_png(&"a".repeat(zterm_core::MAX_TICKET_TEXT_BYTES), &path).is_err());
        assert!(!path.exists());
    }
}
