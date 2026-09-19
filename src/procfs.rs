// Author: Jeff
// Date: 2026-09-18
// Description: Readers for the Linux /proc files the task manager needs
// Notes: Every parser takes plain text so tests can feed it fixtures; the read_* helpers
//        take a root path so tests can point them at a fake /proc. A process can vanish between
//        listing and reading it, so every read returns Option and a missing file is not an error

use std::collections::HashMap;
use std::fs;
use std::path::Path;

// ── One process, as /proc describes it at one moment ─────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct ProcStat {
    pub pid: u32,
    pub comm: String,
    pub state: char,
    pub ppid: u32,
    pub utime: u64,
    pub stime: u64,
    pub nice: i32,
    pub threads: u32,
    pub start_ticks: u64,
}

// Parse /proc/<pid>/stat
// the name sits in parentheses and may itself hold spaces or ')' — so split at the LAST ')'
pub fn parse_stat(text: &str) -> Option<ProcStat> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let pid = text[..open].trim().parse().ok()?;
    let comm = text[open + 1..close].to_string();
    let rest: Vec<&str> = text[close + 1..].split_whitespace().collect();
    // fields after the name start at field 3 (state); index = field number - 3
    let field = |n: usize| rest.get(n - 3).copied();
    Some(ProcStat {
        pid,
        comm,
        state: field(3)?.chars().next()?,
        ppid: field(4)?.parse().ok()?,
        utime: field(14)?.parse().ok()?,
        stime: field(15)?.parse().ok()?,
        nice: field(19)?.parse().ok()?,
        threads: field(20)?.parse().ok()?,
        start_ticks: field(22)?.parse().ok()?,
    })
}

// Parse the owner uid and resident memory (bytes) out of /proc/<pid>/status
pub fn parse_status(text: &str) -> (Option<u32>, Option<u64>) {
    let mut uid = None;
    let mut rss = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("Uid:") {
            uid = v.split_whitespace().next().and_then(|s| s.parse().ok());
        } else if let Some(v) = line.strip_prefix("VmRSS:") {
            rss = v
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<u64>().ok())
                .map(|kb| kb * 1024);
        }
    }
    (uid, rss)
}

// Parse bytes read and written from /proc/<pid>/io (only readable for your own processes)
pub fn parse_io(text: &str) -> Option<(u64, u64)> {
    let mut read = None;
    let mut written = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("read_bytes:") {
            read = v.trim().parse().ok();
        } else if let Some(v) = line.strip_prefix("write_bytes:") {
            written = v.trim().parse().ok();
        }
    }
    Some((read?, written?))
}

// Turn /proc/<pid>/cmdline (NUL-separated) into a readable command line
pub fn parse_cmdline(bytes: &[u8]) -> String {
    bytes
        .split(|b| *b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

// Name the app a process belongs to from its cgroup path
// systemd puts launched apps in app-<launcher>-<name>-<id>.scope and services in <name>.service;
// strip the launcher prefix and the unique id so every window of one app groups together
pub fn app_from_cgroup(text: &str) -> Option<String> {
    let path = cgroup_path(text)?;
    let leaf = path.rsplit('/').next()?;
    if let Some(unit) = leaf.strip_suffix(".service") {
        // app-*.service is a launched app too (uwsm style); plain services keep their name
        return Some(clean_app(unit.strip_prefix("app-").unwrap_or(unit)));
    }
    let scope = leaf.strip_suffix(".scope")?.strip_prefix("app-")?;
    Some(clean_app(scope))
}

// The unified (cgroup v2) path from /proc/<pid>/cgroup — the "0::" line
pub fn cgroup_path(text: &str) -> Option<&str> {
    text.lines().find_map(|l| l.strip_prefix("0::"))
}

// A scope that holds one launched program but carries no app name (systemd-run's run-p<pid>-…)
// its processes belong together, named after whichever one started it.
// Login sessions and init are scopes too, but hold unrelated programs — never grouped
pub fn is_unnamed_scope(path: &str) -> bool {
    let leaf = path.rsplit('/').next().unwrap_or(path);
    leaf.ends_with(".scope")
        && !leaf.starts_with("app-")
        && !leaf.starts_with("session-")
        && leaf != "init.scope"
}

// Drop launcher prefixes and trailing ids: "hyprland-firefox-1234" → "firefox"
fn clean_app(name: &str) -> String {
    let name = name.split('@').next().unwrap_or(name);
    let mut parts: Vec<&str> = name.split('-').collect();
    while parts.len() > 1
        && parts
            .last()
            .is_some_and(|p| p.chars().all(|c| c.is_ascii_hexdigit()))
    {
        parts.pop();
    }
    for launcher in ["hyprland", "uwsm", "flatpak", "dbus", "gnome"] {
        if parts.len() > 1 && parts[0] == launcher {
            parts.remove(0);
        }
    }
    // Ghostty names its windows "<app>-surface-transient-<n>"
    if let Some(i) = parts
        .iter()
        .position(|p| *p == "surface" || *p == "transient")
    {
        parts.truncate(i.max(1));
    }
    parts.join("-")
}

// Total GPU busy time (ns) per DRM client from one fdinfo file: (client id, busy ns)
// several file descriptors can share a client, so the caller dedupes by id
pub fn parse_drm_fdinfo(text: &str) -> Option<(u64, u64)> {
    let mut client = None;
    let mut busy = 0u64;
    let mut seen_engine = false;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("drm-client-id:") {
            client = v.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("drm-engine-")
            && let Some((_, value)) = rest.split_once(':')
            && let Some(ns) = value
                .trim()
                .strip_suffix("ns")
                .and_then(|v| v.trim().parse::<u64>().ok())
        {
            busy += ns;
            seen_engine = true;
        }
    }
    if seen_engine {
        Some((client?, busy))
    } else {
        None
    }
}

// User names by uid from /etc/passwd text
pub fn parse_passwd(text: &str) -> HashMap<u32, String> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split(':');
            let name = fields.next()?;
            let uid = fields.nth(1)?.parse().ok()?;
            Some((uid, name.to_string()))
        })
        .collect()
}

// ── Reading from a /proc tree ────────────────────────────────────────────

// Every numeric directory under the proc root: the pids alive right now
pub fn list_pids(root: &Path) -> Vec<u32> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut pids: Vec<u32> = entries
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
        .collect();
    pids.sort_unstable();
    pids
}

// Read a file under the proc root as text, or None when gone or unreadable
pub fn read_text(root: &Path, rel: &str) -> Option<String> {
    fs::read_to_string(root.join(rel)).ok()
}

// Summed GPU busy ns for one process, deduped across fds that share a DRM client
pub fn read_gpu_busy(root: &Path, pid: u32) -> Option<u64> {
    let dir = fs::read_dir(root.join(format!("{pid}/fdinfo"))).ok()?;
    let mut clients: HashMap<u64, u64> = HashMap::new();
    for entry in dir.flatten() {
        if let Ok(text) = fs::read_to_string(entry.path())
            && let Some((client, busy)) = parse_drm_fdinfo(&text)
        {
            clients.insert(client, busy);
        }
    }
    if clients.is_empty() {
        None
    } else {
        Some(clients.values().sum())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = "2744688 (zsh (odd) name) S 3532360 2744688 2744688 0 -1 4194304 333 1330 0 3 7 5 3 1 20 -5 4 0 27098469 9699328 1612 18446744073709551615";

    #[test]
    fn stat_splits_at_the_last_paren_so_odd_names_survive() {
        let s = parse_stat(STAT).expect("parses");
        assert_eq!(s.pid, 2744688);
        assert_eq!(s.comm, "zsh (odd) name");
        assert_eq!(s.state, 'S');
        assert_eq!(s.ppid, 3532360);
        assert_eq!((s.utime, s.stime), (7, 5));
        assert_eq!(s.nice, -5);
        assert_eq!(s.threads, 4);
        assert_eq!(s.start_ticks, 27098469);
        assert!(parse_stat("garbage").is_none());
    }

    #[test]
    fn status_gives_owner_and_resident_memory() {
        let (uid, rss) =
            parse_status("Name:\tzsh\nUid:\t1000\t1000\t1000\t1000\nVmRSS:\t    6568 kB\n");
        assert_eq!(uid, Some(1000));
        assert_eq!(rss, Some(6568 * 1024));
        // kernel threads have no VmRSS line
        assert_eq!(parse_status("Uid:\t0\t0\t0\t0\n").1, None);
    }

    #[test]
    fn io_needs_both_counters() {
        assert_eq!(
            parse_io("rchar: 1\nread_bytes: 843776\nwrite_bytes: 4096\n"),
            Some((843776, 4096))
        );
        assert_eq!(parse_io("read_bytes: 1\n"), None);
    }

    #[test]
    fn cmdline_joins_arguments() {
        assert_eq!(
            parse_cmdline(b"/usr/bin/foot\0--app-id\0x\0"),
            "/usr/bin/foot --app-id x"
        );
        assert_eq!(parse_cmdline(b""), "");
    }

    #[test]
    fn cgroups_name_the_app_a_process_belongs_to() {
        let cg = |leaf: &str| {
            app_from_cgroup(&format!(
                "0::/user.slice/user-1000.slice/user@1000.service/app.slice/{leaf}\n"
            ))
        };
        assert_eq!(
            cg("app-ghostty-surface-transient-3472769.scope").as_deref(),
            Some("ghostty")
        );
        assert_eq!(
            cg("app-hyprland-firefox-1a2b3c.scope").as_deref(),
            Some("firefox")
        );
        assert_eq!(
            cg("quickshell-mgeist.service").as_deref(),
            Some("quickshell-mgeist")
        );
        assert_eq!(
            cg("app-flatpak-com.spotify.Client-4242.scope").as_deref(),
            Some("com.spotify.Client")
        );
        assert_eq!(app_from_cgroup("0::/init.scope\n"), None);
        assert!(is_unnamed_scope(
            "/user.slice/user@1000.service/app.slice/run-p415401-i408647.scope"
        ));
        assert!(!is_unnamed_scope(
            "/user.slice/user-1000.slice/session-1.scope"
        ));
        assert!(!is_unnamed_scope("/app.slice/app-hyprland-firefox-1.scope"));
        assert!(!is_unnamed_scope("/system.slice/sshd.service"));
    }

    #[test]
    fn gpu_busy_is_summed_over_engines_per_client() {
        let text = "drm-driver:\ti915\ndrm-client-id:\t73\ndrm-engine-render:\t5051599603973 ns\ndrm-engine-copy:\t0 ns\ndrm-engine-video:\t10 ns\n";
        assert_eq!(parse_drm_fdinfo(text), Some((73, 5051599603983)));
        assert_eq!(parse_drm_fdinfo("pos:\t0\nflags:\t02\n"), None);
    }

    #[test]
    fn passwd_maps_uids_to_names() {
        let users = parse_passwd(
            "root:x:0:0::/root:/bin/bash\nmgeist:x:1000:1000::/home/mgeist:/bin/zsh\nbroken\n",
        );
        assert_eq!(users.get(&1000).map(String::as_str), Some("mgeist"));
        assert_eq!(users.len(), 2);
    }
}
