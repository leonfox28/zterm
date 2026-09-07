//! Bounded input admission while a healthy attachment returns from history.

/// Maximum retained input shared with the desktop's input epoch/codec owners.
pub const RESUME_INPUT_BOUND: usize = 1024 * 1024 - 1024;

/// Admission failed without appending any part of the incoming input unit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResumeInputOverflow;

/// Appends one complete encoded input unit atomically under the shared bound.
/// This admits only same-attachment resume input, never disconnected replay.
pub fn append_resume_input(
    retained: &mut Vec<u8>,
    bytes: &[u8],
) -> Result<(), ResumeInputOverflow> {
    let combined = retained
        .len()
        .checked_add(bytes.len())
        .ok_or(ResumeInputOverflow)?;
    if combined > RESUME_INPUT_BOUND {
        return Err(ResumeInputOverflow);
    }
    retained.extend_from_slice(bytes);
    Ok(())
}
