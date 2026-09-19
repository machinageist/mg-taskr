// Author: Jeff
// Date: 2026-09-18
// Description: Two looks at /proc a moment apart → per-process CPU, memory, disk and GPU use
// Notes: Rates need two samples: CPU % is the CPU time a process used between them divided by
//        the wall time between them. `cpu_share` is out of the whole machine (all cores = 100%,
//        like Windows Task Manager); `cpu_core` is out of one core (like top). Disk and GPU
//        counters are only readable for your own processes; others report None

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use serde::Serialize;

use crate::procfs;

// ── Raw counters for one process at one moment ───────────────────────────

#[derive(Debug, Clone)]
pub struct Raw {
    pub stat: procfs::ProcStat,
    pub uid: Option<u32>,
    pub rss: Option<u64>,
    pub io: Option<(u64, u64)>,
    pub gpu_busy_ns: Option<u64>,
    pub cmdline: String,
    pub app: Option<String>,
}

pub struct Snapshot {
    pub at: Instant,
    pub procs: HashMap<u32, Raw>,
}

// Read every visible process under the proc root
// a process that exits halfway through reading is simply left out
pub fn snapshot(root: &Path) -> Snapshot {
    let at = Instant::now();
    let mut procs = HashMap::new();
    for pid in procfs::list_pids(root) {
        let Some(stat) =
            procfs::read_text(root, &format!("{pid}/stat")).and_then(|t| procfs::parse_stat(&t))
        else {
            continue;
        };
        let (uid, rss) = procfs::read_text(root, &format!("{pid}/status"))
            .map(|t| procfs::parse_status(&t))
            .unwrap_or((None, None));
        let io = procfs::read_text(root, &format!("{pid}/io")).and_then(|t| procfs::parse_io(&t));
        let cmdline = std::fs::read(root.join(format!("{pid}/cmdline")))
            .map(|b| procfs::parse_cmdline(&b))
            .unwrap_or_default();
        let app = procfs::read_text(root, &format!("{pid}/cgroup"))
            .and_then(|t| procfs::app_from_cgroup(&t));
        let gpu_busy_ns = procfs::read_gpu_busy(root, pid);
        procs.insert(
            pid,
            Raw {
                stat,
                uid,
                rss,
                io,
                gpu_busy_ns,
                cmdline,
                app,
            },
        );
    }
    Snapshot { at, procs }
}

// ── One row the CLI, TUI and shell show ──────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct Process {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub command: String,
    pub user: String,
    pub uid: Option<u32>,
    pub state: String,
    pub threads: u32,
    pub nice: i32,
    pub cpu_share: f64,
    pub cpu_core: f64,
    pub memory: u64,
    pub read_rate: Option<f64>,
    pub write_rate: Option<f64>,
    pub gpu: Option<f64>,
    pub app: String,
    pub mine: bool,
}

// Plain words for the one-letter process states
pub fn state_name(state: char) -> &'static str {
    match state {
        'R' => "running",
        'S' => "sleeping",
        'D' => "waiting on disk",
        'Z' => "zombie",
        'T' | 't' => "stopped",
        'I' => "idle",
        _ => "other",
    }
}

// Compare two snapshots into rows
// `ticks_per_second` is the kernel clock (usually 100); `cores` scales the machine share
pub fn diff(
    before: &Snapshot,
    after: &Snapshot,
    ticks_per_second: f64,
    cores: f64,
    users: &HashMap<u32, String>,
    me: u32,
) -> Vec<Process> {
    let seconds = after.at.duration_since(before.at).as_secs_f64().max(0.001);
    let mut rows = Vec::with_capacity(after.procs.len());
    for (pid, now) in &after.procs {
        // a pid reused by a new process between samples has a different start time — treat as new
        let prev = before
            .procs
            .get(pid)
            .filter(|p| p.stat.start_ticks == now.stat.start_ticks);
        let ticks = prev.map_or(0, |p| {
            (now.stat.utime + now.stat.stime).saturating_sub(p.stat.utime + p.stat.stime)
        });
        let cpu_core = ticks as f64 / ticks_per_second / seconds * 100.0;
        let rate = |pick: fn(&(u64, u64)) -> u64| match (prev.and_then(|p| p.io), now.io) {
            (Some(a), Some(b)) => Some(pick(&b).saturating_sub(pick(&a)) as f64 / seconds),
            _ => None,
        };
        let gpu = match (prev.and_then(|p| p.gpu_busy_ns), now.gpu_busy_ns) {
            (Some(a), Some(b)) => {
                Some((b.saturating_sub(a) as f64 / (seconds * 1e9) * 100.0).min(100.0))
            }
            _ => None,
        };
        let name = if now.stat.comm.is_empty() {
            format!("pid {pid}")
        } else {
            now.stat.comm.clone()
        };
        rows.push(Process {
            pid: *pid,
            ppid: now.stat.ppid,
            command: if now.cmdline.is_empty() {
                format!("[{name}]")
            } else {
                now.cmdline.clone()
            },
            user: now
                .uid
                .and_then(|u| users.get(&u).cloned())
                .unwrap_or_else(|| now.uid.map_or("?".into(), |u| u.to_string())),
            uid: now.uid,
            state: state_name(now.stat.state).to_string(),
            threads: now.stat.threads,
            nice: now.stat.nice,
            cpu_share: cpu_core / cores.max(1.0),
            cpu_core,
            memory: now.rss.unwrap_or(0),
            read_rate: rate(|io| io.0),
            write_rate: rate(|io| io.1),
            gpu,
            app: now.app.clone().unwrap_or_else(|| name.clone()),
            mine: now.uid == Some(me),
            name,
        });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn raw(pid: u32, ticks: u64, start: u64, io: Option<(u64, u64)>, gpu: Option<u64>) -> Raw {
        Raw {
            stat: procfs::ProcStat {
                pid,
                comm: "app".into(),
                state: 'S',
                ppid: 1,
                utime: ticks,
                stime: 0,
                nice: 0,
                threads: 1,
                start_ticks: start,
            },
            uid: Some(1000),
            rss: Some(4096),
            io,
            gpu_busy_ns: gpu,
            cmdline: "app --flag".into(),
            app: Some("app".into()),
        }
    }

    fn snap(at: Instant, raws: Vec<Raw>) -> Snapshot {
        Snapshot {
            at,
            procs: raws.into_iter().map(|r| (r.stat.pid, r)).collect(),
        }
    }

    #[test]
    fn cpu_io_and_gpu_rates_come_from_the_difference() {
        let t0 = Instant::now();
        let before = snap(t0, vec![raw(10, 100, 5, Some((0, 0)), Some(0))]);
        // one second later: 50 ticks at 100/s = half a core; 2 MB read; GPU busy 0.25 s
        let after = snap(
            t0 + Duration::from_secs(1),
            vec![raw(10, 150, 5, Some((2_000_000, 1000)), Some(250_000_000))],
        );
        let users = HashMap::from([(1000, "mgeist".to_string())]);
        let row = &diff(&before, &after, 100.0, 4.0, &users, 1000)[0];
        assert!((row.cpu_core - 50.0).abs() < 1e-6);
        assert!(
            (row.cpu_share - 12.5).abs() < 1e-6,
            "half a core of four is 12.5% of the machine"
        );
        assert_eq!(row.read_rate.map(|r| r.round()), Some(2_000_000.0));
        assert_eq!(row.gpu.map(|g| g.round()), Some(25.0));
        assert_eq!(row.user, "mgeist");
        assert!(row.mine);
    }

    #[test]
    fn a_reused_pid_is_a_new_process_not_a_spike() {
        let t0 = Instant::now();
        let before = snap(t0, vec![raw(10, 100, 5, None, None)]);
        let after = snap(
            t0 + Duration::from_secs(1),
            vec![raw(10, 900, 99, None, None)],
        );
        let row = &diff(&before, &after, 100.0, 1.0, &HashMap::new(), 0)[0];
        assert_eq!(row.cpu_core, 0.0);
        assert_eq!(row.user, "1000", "an unknown uid shows as its number");
    }

    #[test]
    fn states_read_as_words() {
        assert_eq!(state_name('D'), "waiting on disk");
        assert_eq!(state_name('?'), "other");
    }
}
