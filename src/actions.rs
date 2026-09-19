// Author: Jeff
// Date: 2026-09-18
// Description: Doing things to processes and services — signal, renice, start/stop/restart
// Notes: Your own processes and user services are handled directly. Anything owned by root
//        or another user goes through the geist root helper (pkexec, passwordless for Jeff's
//        session), which refuses unless /etc/geist/taskr.conf lists that unit or program.
//        Renice is own-processes only: making something slower is allowed, making it faster
//        needs root, and the helper deliberately cannot do that

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::{os, procfs};

const ROOT_HELPER: &str = "/usr/local/libexec/geist-root-helper";
const PKEXEC: &str = "/usr/bin/pkexec";
const SYSTEMCTL: &str = "systemctl";
const INSTALL_HINT: &str =
    "system actions need the one-time root helper: sudo ~/dotfiles/system/install-root-helper.sh";
const MAX_UNIT_LEN: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Signal {
    /// Ask it to quit (SIGTERM)
    Term,
    /// Force it to quit (SIGKILL)
    Kill,
    /// Pause it (SIGSTOP)
    Stop,
    /// Resume a paused process (SIGCONT)
    Cont,
}

impl Signal {
    // The kernel's number for this signal
    pub fn number(self) -> i32 {
        match self {
            Signal::Term => libc::SIGTERM,
            Signal::Kill => libc::SIGKILL,
            Signal::Stop => libc::SIGSTOP,
            Signal::Cont => libc::SIGCONT,
        }
    }

    // The word the CLI and the root helper use
    pub fn word(self) -> &'static str {
        match self {
            Signal::Term => "term",
            Signal::Kill => "kill",
            Signal::Stop => "stop",
            Signal::Cont => "cont",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ServiceVerb {
    Start,
    Stop,
    Restart,
}

impl ServiceVerb {
    // The systemctl verb
    pub fn word(self) -> &'static str {
        match self {
            ServiceVerb::Start => "start",
            ServiceVerb::Stop => "stop",
            ServiceVerb::Restart => "restart",
        }
    }
}

// Who owns a process and what it is called, straight from /proc
fn owner(proc_root: &Path, pid: u32) -> Result<(u32, String)> {
    let status = procfs::read_text(proc_root, &format!("{pid}/status"))
        .with_context(|| format!("no process {pid}"))?;
    let (uid, _) = procfs::parse_status(&status);
    let name = status
        .lines()
        .find_map(|l| l.strip_prefix("Name:"))
        .unwrap_or("")
        .trim()
        .to_string();
    Ok((
        uid.with_context(|| format!("cannot tell who owns {pid}"))?,
        name,
    ))
}

// Send a signal: directly to our own process, through the helper for anyone else's
pub fn signal(proc_root: &Path, pid: u32, sig: Signal) -> Result<String> {
    let (uid, name) = owner(proc_root, pid)?;
    if uid != os::my_uid() {
        return helper(&["taskr-signal", &pid.to_string(), sig.word()]);
    }
    os::send_signal(pid, sig.number()).with_context(|| format!("{} {pid} ({name})", sig.word()))?;
    Ok(format!("{} sent to {pid} ({name})", sig.word()))
}

// Signal several processes (an app), trying every one even if some refuse
// one pid reports exactly what went wrong; several report the first failure plus a count
pub fn signal_many(proc_root: &Path, pids: &[u32], sig: Signal, label: &str) -> Result<String> {
    let mut failed: Vec<anyhow::Error> = Vec::new();
    for pid in pids {
        if let Err(e) = signal(proc_root, *pid, sig) {
            failed.push(e);
        }
    }
    let count = failed.len();
    match failed.into_iter().next() {
        None if pids.len() == 1 => Ok(format!("{} sent to {label}", sig.word())),
        None => Ok(format!(
            "{} sent to {label} ({} processes)",
            sig.word(),
            pids.len()
        )),
        Some(e) if pids.len() == 1 => Err(e),
        Some(e) => Err(e.context(format!(
            "{} {label}: {count} of {} processes refused",
            sig.word(),
            pids.len()
        ))),
    }
}

// Change the nice value of one of our own processes
pub fn renice(proc_root: &Path, pid: u32, nice: i32) -> Result<String> {
    let (uid, name) = owner(proc_root, pid)?;
    if uid != os::my_uid() {
        bail!("only your own processes can be reniced — {name} ({pid}) belongs to uid {uid}");
    }
    match os::set_nice(pid, nice) {
        Ok(()) => Ok(format!("{name} ({pid}) now at nice {nice}")),
        // lowering nice below where it is needs root, and the helper does not offer it
        Err(e)
            if e.raw_os_error() == Some(libc::EACCES) || e.raw_os_error() == Some(libc::EPERM) =>
        {
            bail!(
                "making {name} faster (nice below its current value) needs root; mg-taskr only slows processes down"
            )
        }
        Err(e) => Err(e).with_context(|| format!("renice {pid}")),
    }
}

// Same rule the root helper applies, checked here first for a clear message
// starts with a letter or digit (never an option), systemd name characters, ends .service
pub fn valid_unit(unit: &str) -> bool {
    let allowed = |c: char| c.is_ascii_alphanumeric() || ":_.@\\-".contains(c);
    unit.len() <= MAX_UNIT_LEN
        && unit.starts_with(|c: char| c.is_ascii_alphanumeric())
        && unit.chars().all(allowed)
        && unit.ends_with(".service")
}

// Start, stop or restart a service — user units directly, system units via the helper
pub fn service(scope: crate::services::Scope, verb: ServiceVerb, unit: &str) -> Result<String> {
    if !valid_unit(unit) {
        bail!("{unit:?} is not a service unit name");
    }
    if scope == crate::services::Scope::System {
        return helper(&["taskr-service", verb.word(), unit]);
    }
    let output = Command::new(SYSTEMCTL)
        .args(["--user", verb.word(), "--", unit])
        .output()
        .context("running systemctl")?;
    if !output.status.success() {
        bail!(
            "systemctl --user {} {unit}: {}",
            verb.word(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(format!("{} {unit}: done", verb.word()))
}

// Run one root-helper action and pass back its answer
fn helper(args: &[&str]) -> Result<String> {
    if !Path::new(ROOT_HELPER).exists() {
        bail!(INSTALL_HINT);
    }
    let output = Command::new(PKEXEC)
        .arg(ROOT_HELPER)
        .args(args)
        .output()
        .context("running pkexec")?;
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr);
        let reason = said.trim().strip_prefix("refused: ").unwrap_or(said.trim());
        bail!(
            "{}",
            if reason.is_empty() {
                "the root helper refused"
            } else {
                reason
            }
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_names_follow_the_helpers_rule() {
        assert!(valid_unit("ollama.service"));
        assert!(valid_unit("app-nm\\x2dapplet@autostart.service"));
        for bad in [
            "",
            "-x.service",
            "a b.service",
            "a;b.service",
            "ollama.socket",
            "../x.service",
            "a.service\n",
        ] {
            assert!(!valid_unit(bad), "{bad:?}");
        }
    }

    #[test]
    fn signals_map_to_kernel_numbers_and_helper_words() {
        assert_eq!(Signal::Kill.number(), 9);
        assert_eq!(Signal::Cont.word(), "cont");
    }

    #[test]
    fn signalling_ourselves_with_cont_works_and_pid_zero_never_reaches_kill() {
        let me = std::process::id();
        let said = signal(Path::new("/proc"), me, Signal::Cont).expect("own process");
        assert!(said.starts_with("cont sent to"));
        assert!(
            os::send_signal(0, 0).is_err(),
            "pid 0 would mean the whole group"
        );
    }

    #[test]
    fn a_missing_process_is_a_plain_error() {
        let root = std::env::temp_dir();
        assert!(signal(&root, 4_000_000, Signal::Term).is_err());
    }

    #[test]
    fn several_pids_are_all_tried_and_failures_counted() {
        let me = std::process::id();
        let said = signal_many(Path::new("/proc"), &[me, me], Signal::Cont, "me").unwrap();
        assert_eq!(said, "cont sent to me (2 processes)");
        let err =
            signal_many(Path::new("/proc"), &[me, 4_000_000], Signal::Cont, "mixed").unwrap_err();
        assert!(format!("{err:#}").starts_with("cont mixed: 1 of 2 processes refused"));
    }
}
