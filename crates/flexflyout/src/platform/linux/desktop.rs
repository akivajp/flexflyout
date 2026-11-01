//! The desktop's conventions: opening files and links with the user's
//! applications (xdg-open), and starting at login (an XDG autostart
//! entry, which KDE Plasma, GNOME and most others read).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context as _;

/// Opens `path` in the application the desktop associates with it (a text
/// editor for the settings file).
pub fn open_in_editor(path: &Path) {
    open(path.as_os_str());
}

/// Opens `url` in the default browser.
pub fn open_url(url: &str) {
    open(url.as_ref());
}

/// Hands `target` to xdg-open, without waiting for it.
fn open(target: &std::ffi::OsStr) {
    match Command::new("xdg-open").arg(target).spawn() {
        // Reap it in the background so that it does not linger as a zombie.
        Ok(mut child) => {
            let _ = std::thread::Builder::new()
                .name("xdg-open".into())
                .spawn(move || child.wait());
        }
        Err(err) => log::warn!("cannot open {}: {err}", target.to_string_lossy()),
    }
}

/// Where the autostart entry goes:
/// `$XDG_CONFIG_HOME/autostart/flexflyout.desktop` (or `~/.config/...`).
fn autostart_file() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("autostart").join("flexflyout.desktop"))
}

/// Whether FlexFlyout starts at login.
pub fn autostart_enabled() -> bool {
    autostart_file()
        .and_then(|path| std::fs::read_to_string(path).ok())
        // Desktops turn an entry off with Hidden=true instead of deleting it.
        .is_some_and(|entry| !entry.lines().any(|line| line.trim() == "Hidden=true"))
}

/// Makes FlexFlyout start at login, reading `config_file` (passed with
/// `--config` when it is not the default one), or stops it from doing so.
pub fn set_autostart(enabled: bool, config_file: &Path) -> anyhow::Result<()> {
    let path = autostart_file().context("no home folder")?;
    if !enabled {
        return match std::fs::remove_file(&path) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                Err(err).context("cannot remove the autostart entry")
            }
            _ => Ok(()),
        };
    }
    let exe = std::env::current_exe().context("cannot tell where this program is")?;
    let mut exec = quote(&exe.to_string_lossy());
    if config_file != crate::settings::default_file() {
        exec.push_str(" --config ");
        exec.push_str(&quote(&config_file.to_string_lossy()));
    }
    let entry = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=FlexFlyout\n\
         Comment=Media flyout: cover art, playback controls and volume\n\
         Exec={exec}\n\
         Icon=flexflyout\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n"
    );
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).context("cannot make the autostart folder")?;
    }
    std::fs::write(&path, entry).context("cannot write the autostart entry")?;
    log::info!("starts at login: {}", path.display());
    Ok(())
}

/// Quotes an argument for a desktop entry's Exec key.
fn quote(argument: &str) -> String {
    let mut quoted = String::from("\"");
    for c in argument.chars() {
        // Characters with a meaning inside quotes are escaped; % starts a
        // field code and is doubled.
        match c {
            '"' | '`' | '$' | '\\' => {
                quoted.push('\\');
                quoted.push(c);
            }
            '%' => quoted.push_str("%%"),
            _ => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_arguments_are_quoted() {
        assert_eq!(quote("/opt/flexflyout"), "\"/opt/flexflyout\"");
        assert_eq!(quote("/home/a b/c\"d"), "\"/home/a b/c\\\"d\"");
        assert_eq!(quote("/x/100%"), "\"/x/100%%\"");
    }
}
