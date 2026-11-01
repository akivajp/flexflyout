//! The desktop's fonts, found through fontconfig's `fc-match` (every Linux
//! desktop has it), so that the flyout looks like the rest of the desktop
//! and shows Japanese titles.

use std::process::Command;

/// The font files to use, primary first: the desktop's sans-serif font,
/// then one for Japanese (titles can be Japanese whatever the language).
/// Each is read whole; a missing one is left out.
pub fn find() -> Vec<(String, Vec<u8>)> {
    let mut found: Vec<(String, Vec<u8>)> = Vec::new();
    for pattern in ["sans-serif", "sans-serif:lang=ja"] {
        let Some(path) = fc_match(pattern) else {
            continue;
        };
        if found.iter().any(|(known, _)| *known == path) {
            continue;
        }
        match std::fs::read(&path) {
            Ok(bytes) => found.push((path, bytes)),
            Err(err) => log::warn!("cannot read the font {path}: {err}"),
        }
    }
    found
}

/// The file fontconfig picks for `pattern`.
fn fc_match(pattern: &str) -> Option<String> {
    let output = Command::new("fc-match")
        .args(["--format=%{file}", pattern])
        .output()
        .inspect_err(|err| log::warn!("cannot run fc-match: {err}"))
        .ok()?;
    let path = String::from_utf8(output.stdout).ok()?;
    (output.status.success() && !path.is_empty()).then_some(path)
}
