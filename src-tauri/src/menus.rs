//! Shell integration: right-click entries for Nautilus and Dolphin.
//!
//! Both file managers only read static files, but the device list is dynamic,
//! so these are regenerated whenever the list changes. Each per-device entry
//! invokes the binary in headless `--send-to` mode, which sends and exits
//! without starting a GUI.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::tailscale::Target;

const SUBMENU: &str = "Send with Chute";

/// How a menu entry should invoke us.
///
/// Inside a Flatpak the host file manager cannot execute our sandboxed binary
/// directly, so entries have to go back through `flatpak run`.
fn launch_command() -> String {
    if let Ok(id) = std::env::var("FLATPAK_ID") {
        return format!("flatpak run {id}");
    }
    std::env::current_exe()
        .map(|p| shell_quote(&p.to_string_lossy()))
        .unwrap_or_else(|_| "chute".into())
}

/// Single-quote for a POSIX shell / Exec= line.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Strip characters that would break a filename or a desktop-entry action id.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Rewrite every integration point. Errors are collected rather than fatal:
/// a missing Dolphin is not a reason to skip Nautilus.
pub fn regenerate(targets: &[Target]) -> Vec<String> {
    let mut errors = Vec::new();
    let cmd = launch_command();

    if let Err(e) = write_nautilus(targets, &cmd) {
        errors.push(format!("nautilus: {e}"));
    }
    if let Err(e) = write_dolphin(targets, &cmd) {
        errors.push(format!("dolphin: {e}"));
    }
    errors
}

/// Nautilus reads executables from a scripts directory; a subdirectory becomes
/// a submenu, which is how the per-device list is expressed.
fn write_nautilus(targets: &[Target], cmd: &str) -> std::io::Result<()> {
    let Some(home) = home() else {
        return Ok(());
    };
    let dir = home.join(".local/share/nautilus/scripts").join(SUBMENU);

    // Rewrite wholesale so devices that disappeared do not linger.
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    fs::create_dir_all(&dir)?;

    write_script(&dir.join("Choose device…"), &format!(
        "#!/bin/sh\n\
         # Selected paths arrive newline-separated, so split on newlines only\n\
         # and disable globbing before word-splitting them.\n\
         IFS='\n'\n\
         set -f\n\
         exec {cmd} $NAUTILUS_SCRIPT_SELECTED_FILE_PATHS\n"
    ))?;

    for t in targets.iter().filter(|t| t.online) {
        let body = format!(
            "#!/bin/sh\n\
             IFS='\n'\n\
             set -f\n\
             exec {cmd} --send-to {} $NAUTILUS_SCRIPT_SELECTED_FILE_PATHS\n",
            shell_quote(&t.name)
        );
        write_script(&dir.join(&t.name), &body)?;
    }
    Ok(())
}

fn write_script(path: &Path, body: &str) -> std::io::Result<()> {
    let mut f = fs::File::create(path)?;
    f.write_all(body.as_bytes())?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

/// Dolphin reads a single desktop file describing a submenu of actions.
///
/// KDE moved the location between Plasma 5 and 6 and the Steam Deck's version
/// depends on the SteamOS release, so write both; the unused one is inert.
fn write_dolphin(targets: &[Target], cmd: &str) -> std::io::Result<()> {
    let Some(home) = home() else {
        return Ok(());
    };

    let online: Vec<&Target> = targets.iter().filter(|t| t.online).collect();
    let mut action_ids: Vec<String> = vec!["choose".into()];
    action_ids.extend(online.iter().map(|t| format!("send_{}", sanitize(&t.name))));

    let mut body = String::from("[Desktop Entry]\nType=Service\nMimeType=all/all;\n");
    body.push_str(&format!("X-KDE-Submenu={SUBMENU}\n"));
    body.push_str("Icon=document-send\n");
    body.push_str(&format!("Actions={};\n", action_ids.join(";")));

    // %F expands to the selected paths, quoted by KDE.
    body.push_str("\n[Desktop Action choose]\nName=Choose device…\n");
    body.push_str(&format!("Exec={cmd} %F\n"));

    for t in &online {
        body.push_str(&format!("\n[Desktop Action send_{}]\n", sanitize(&t.name)));
        body.push_str(&format!("Name={}\n", t.name));
        body.push_str(&format!("Exec={cmd} --send-to {} %F\n", shell_quote(&t.name)));
    }

    for rel in [".local/share/kio/servicemenus", ".local/share/kservices5/ServiceMenus"] {
        let dir = home.join(rel);
        fs::create_dir_all(&dir)?;
        let path = dir.join("chute.desktop");
        fs::write(&path, &body)?;
        // KF6 requires service menu files to be executable.
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

/// Remove everything we installed.
pub fn remove() {
    let Some(home) = home() else { return };
    let _ = fs::remove_dir_all(home.join(".local/share/nautilus/scripts").join(SUBMENU));
    for rel in [".local/share/kio/servicemenus", ".local/share/kservices5/ServiceMenus"] {
        let _ = fs::remove_file(home.join(rel).join("chute.desktop"));
    }
}
