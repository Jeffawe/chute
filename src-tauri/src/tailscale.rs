//! Thin wrapper over the `tailscale` CLI.
//!
//! Everything here is pinned to behaviour measured against tailscale 1.102.3:
//!
//!   * `file cp --targets` prints `IP \t name`, plus a third column that only
//!     appears for offline peers (`offline; last seen -3m0s ago`).
//!   * `file cp` is silent on success and exits 1 with one human-readable
//!     stderr line on failure.
//!   * `file get --verbose` prints `wrote <orig> as <final> (<n> bytes)` and
//!     never identifies the sending device.
//!
//! Keeping every invocation in this module means a change to any of those
//! formats is a one-file fix.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::process::{Child, Command};

/// Inside the Flatpak we ship our own statically linked copy, because the
/// sandbox cannot see the host's `/usr/bin`. On a plain host build we just use
/// whatever is on PATH.
fn binary() -> &'static Path {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        if let Some(p) = std::env::var_os("TAILDROP_TS_BIN") {
            return PathBuf::from(p);
        }
        let bundled = PathBuf::from("/app/bin/tailscale");
        if bundled.is_file() {
            return bundled;
        }
        PathBuf::from("tailscale")
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub ip: String,
    pub name: String,
    pub online: bool,
    /// Raw status text from the third column, only present when offline.
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Received {
    pub name: String,
    pub bytes: u64,
    /// Full path on disk, so the UI can reveal it in a file manager.
    pub path: PathBuf,
}

#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum Error {
    #[error("the tailscale CLI could not be found")]
    MissingBinary,
    #[error("tailscaled is not running or could not be reached")]
    DaemonUnreachable,
    #[error("{0}")]
    Cli(String),
    #[error("{0}")]
    Io(String),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::NotFound => Error::MissingBinary,
            _ => Error::Io(e.to_string()),
        }
    }
}

/// Turn a failed invocation's stderr into something the UI can act on.
fn classify(stderr: &str) -> Error {
    let msg = stderr.trim();
    let lowered = msg.to_ascii_lowercase();
    if lowered.contains("is tailscaled running")
        || lowered.contains("failed to connect")
        || lowered.contains("connection refused")
        || lowered.contains("no such file or directory")
            && lowered.contains("tailscaled.sock")
    {
        return Error::DaemonUnreachable;
    }
    if msg.is_empty() {
        return Error::Cli("tailscale exited with an error".into());
    }
    // The CLI is chatty on some failures; the first line is the useful one.
    Error::Cli(msg.lines().next().unwrap_or(msg).to_string())
}

/// Devices this machine can currently send to.
pub async fn targets() -> Result<Vec<Target>, Error> {
    let out = Command::new(binary())
        .args(["file", "cp", "--targets"])
        .output()
        .await?;

    if !out.status.success() {
        return Err(classify(&String::from_utf8_lossy(&out.stderr)));
    }

    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(parse_target)
        .collect())
}

fn parse_target(line: &str) -> Option<Target> {
    let mut cols = line.split('\t');
    let ip = cols.next()?.trim().to_string();
    let name = cols.next()?.trim().to_string();
    if ip.is_empty() || name.is_empty() {
        return None;
    }
    let status = cols
        .next()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    Some(Target {
        ip,
        name,
        online: status.is_none(),
        status,
    })
}

/// Send files to a peer. `target` is the bare device name; the trailing colon
/// the CLI insists on is added here so callers can't forget it.
pub async fn send(paths: &[PathBuf], target: &str) -> Result<(), Error> {
    if paths.is_empty() {
        return Err(Error::Cli("no files selected".into()));
    }

    let mut cmd = Command::new(binary());
    cmd.args(["file", "cp"]);
    // Passed as argv, never through a shell, so spaces and parentheses in
    // filenames need no quoting of our own.
    for p in paths {
        cmd.arg(p);
    }
    cmd.arg(format!("{target}:"));

    let out = cmd.output().await?;
    if out.status.success() {
        Ok(())
    } else {
        Err(classify(&String::from_utf8_lossy(&out.stderr)))
    }
}

/// Start a long-running receiver that drains the inbox into `dir` as files
/// arrive. Caller owns the child and reads its stdout.
///
/// Orphaning this child is worse than it sounds: a stray `file get --loop`
/// keeps draining the same inbox, so the next launch would have two receivers
/// racing for every incoming file. `kill_on_drop` handles an orderly shutdown,
/// but destructors do not run when the app is killed outright, so we also ask
/// the kernel to take the child down with us.
pub fn spawn_receiver(dir: &Path) -> Result<Child, Error> {
    let mut cmd = Command::new(binary());
    cmd.args(["file", "get", "--loop", "--verbose", "--conflict=rename"])
        .arg(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: prctl is async-signal-safe and touches only this process.
        unsafe {
            cmd.as_std_mut().pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    cmd.spawn().map_err(Into::into)
}

/// True if `pid` is a receiver we started. Guards against a recycled PID by
/// checking the command line rather than trusting the number alone.
pub fn is_receiver(pid: u32) -> bool {
    let Ok(raw) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
        return false;
    };
    let args: Vec<String> = raw
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    args.first().is_some_and(|a| a.ends_with("tailscale"))
        && args.iter().any(|a| a == "get")
        && args.iter().any(|a| a == "--loop")
}

/// Pull an arrival out of the receiver's verbose output.
///
/// The observed line is `wrote <orig> as <final> (<n> bytes)`, and it can be
/// preceded on the same line by `waiting for file...`, so this searches rather
/// than anchoring. Both names may contain spaces; the greedy first group makes
/// the split fall on the last ` as `, which is the best available guess.
pub fn parse_received(line: &str, dir: &Path) -> Option<Received> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"wrote (.+) as (.+) \((\d+) bytes\)").unwrap());
    let caps = re.captures(line)?;
    // The second capture is the name actually written, which differs from the
    // first whenever --conflict=rename had to step in.
    let name = caps.get(2)?.as_str().to_string();
    Some(Received {
        bytes: caps.get(3)?.as_str().parse().ok()?,
        path: dir.join(&name),
        name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_online_and_offline_targets() {
        let online = parse_target("100.104.155.44\tipad161").unwrap();
        assert_eq!(online.name, "ipad161");
        assert!(online.online);
        assert!(online.status.is_none());

        let offline =
            parse_target("100.95.234.109\tjeffawe\toffline; last seen -3m0s ago").unwrap();
        assert_eq!(offline.name, "jeffawe");
        assert!(!offline.online);
        assert_eq!(offline.status.as_deref(), Some("offline; last seen -3m0s ago"));
    }

    #[test]
    fn ignores_blank_lines() {
        assert!(parse_target("").is_none());
        assert!(parse_target("100.1.1.1").is_none());
    }

    #[test]
    fn parses_arrival_with_spaces_and_parens() {
        // Exactly what Phase 0 saw come off the wire.
        let line = "waiting for file...wrote image (14) 2.png as image (14) 2.png (181072 bytes)";
        let got = parse_received(line, Path::new("/home/me/Downloads")).unwrap();
        assert_eq!(got.name, "image (14) 2.png");
        assert_eq!(got.bytes, 181072);
        assert_eq!(got.path, Path::new("/home/me/Downloads/image (14) 2.png"));
    }

    #[test]
    fn uses_the_renamed_file_not_the_original() {
        let line = "wrote report.pdf as report (1).pdf (99 bytes)";
        let got = parse_received(line, Path::new("/tmp")).unwrap();
        assert_eq!(got.name, "report (1).pdf");
        assert_eq!(got.path, Path::new("/tmp/report (1).pdf"));
    }

    #[test]
    fn ignores_non_arrival_lines() {
        let d = Path::new("/tmp");
        assert!(parse_received("moved 1/1 files", d).is_none());
        assert!(parse_received("waiting for file...", d).is_none());
    }
}
