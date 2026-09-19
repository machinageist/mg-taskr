// Author: Jeff
// Date: 2026-09-18
// Description: Machine-wide totals — CPU per core, memory and swap, network, disk I/O, load
// Notes: Same two-look idea as sample.rs: /proc only keeps running totals, so a rate is the
//        change between two snapshots divided by the time between them.
//        Disk totals count whole disks only (from /sys/block); partitions would count every
//        byte twice. loop, ram and zram devices are memory or files, not disks, and are skipped

use std::path::Path;
use std::time::Instant;

use serde::Serialize;

use crate::procfs::read_text;

// bytes per sector in /proc/diskstats — always 512, whatever the disk's real sector size
const SECTOR_BYTES: u64 = 512;
const KIB: u64 = 1024;
const NOT_DISKS: [&str; 3] = ["loop", "ram", "zram"];

// ── Parsers ──────────────────────────────────────────────────────────────

// CPU time as busy vs total ticks; idle and iowait both count as not busy
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CpuTimes {
    pub busy: u64,
    pub total: u64,
}

// /proc/stat cpu lines → [whole machine, core 0, core 1, …]
pub fn parse_cpu_times(text: &str) -> Vec<CpuTimes> {
    text.lines()
        .filter(|line| line.starts_with("cpu"))
        .map(|line| {
            let fields: Vec<u64> = line
                .split_whitespace()
                .skip(1)
                .filter_map(|f| f.parse().ok())
                .collect();
            // guest time is already inside user time — summing all ten would count it twice
            let total: u64 = fields.iter().take(8).sum();
            let idle = fields.get(3).copied().unwrap_or(0) + fields.get(4).copied().unwrap_or(0);
            CpuTimes {
                busy: total.saturating_sub(idle),
                total,
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Memory {
    pub total: u64,
    pub available: u64,
    pub used: u64,
    pub cached: u64,
    pub swap_total: u64,
    pub swap_used: u64,
}

// /proc/meminfo → bytes
// used = total − available, the same "in use" the kernel and free(1) mean
pub fn parse_meminfo(text: &str) -> Memory {
    let field = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix(':'))
            .and_then(|rest| rest.split_whitespace().next()?.parse::<u64>().ok())
            .unwrap_or(0)
            * KIB
    };
    let total = field("MemTotal");
    let available = field("MemAvailable");
    let swap_total = field("SwapTotal");
    Memory {
        total,
        available,
        used: total.saturating_sub(available),
        cached: field("Cached") + field("Buffers"),
        swap_total,
        swap_used: swap_total.saturating_sub(field("SwapFree")),
    }
}

// /proc/net/dev → (interface, received bytes, sent bytes), loopback left out
pub fn parse_net_dev(text: &str) -> Vec<(String, u64, u64)> {
    text.lines()
        .filter_map(|line| {
            let (name, rest) = line.split_once(':')?;
            let name = name.trim();
            let fields: Vec<u64> = rest
                .split_whitespace()
                .filter_map(|f| f.parse().ok())
                .collect();
            (name != "lo" && fields.len() >= 9).then(|| (name.to_string(), fields[0], fields[8]))
        })
        .collect()
}

// /proc/diskstats → (disk, bytes read, bytes written) for the named disks only
pub fn parse_diskstats(text: &str, disks: &[String]) -> Vec<(String, u64, u64)> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            let name = *f.get(2)?;
            if !disks.iter().any(|d| d == name) {
                return None;
            }
            let read: u64 = f.get(5)?.parse().ok()?;
            let written: u64 = f.get(9)?.parse().ok()?;
            Some((
                name.to_string(),
                read * SECTOR_BYTES,
                written * SECTOR_BYTES,
            ))
        })
        .collect()
}

// /proc/loadavg → the 1, 5 and 15 minute averages
pub fn parse_loadavg(text: &str) -> [f64; 3] {
    let mut out = [0.0; 3];
    for (slot, field) in out.iter_mut().zip(text.split_whitespace()) {
        *slot = field.parse().unwrap_or(0.0);
    }
    out
}

// Whole disks from /sys/block, minus the ones that are not really disks
pub fn whole_disks(sys_root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(sys_root.join("block")) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|name| !NOT_DISKS.iter().any(|skip| name.starts_with(skip)))
        .collect();
    names.sort();
    names
}

// ── One look at the machine ──────────────────────────────────────────────

pub struct SystemSnapshot {
    pub at: Instant,
    pub cpus: Vec<CpuTimes>,
    pub memory: Memory,
    pub net: Vec<(String, u64, u64)>,
    pub disks: Vec<(String, u64, u64)>,
    pub load: [f64; 3],
    pub uptime_seconds: f64,
}

// Read every machine-wide total at once so the rates line up
pub fn snapshot(proc_root: &Path, sys_root: &Path) -> SystemSnapshot {
    let text = |rel: &str| read_text(proc_root, rel).unwrap_or_default();
    SystemSnapshot {
        at: Instant::now(),
        cpus: parse_cpu_times(&text("stat")),
        memory: parse_meminfo(&text("meminfo")),
        net: parse_net_dev(&text("net/dev")),
        disks: parse_diskstats(&text("diskstats"), &whole_disks(sys_root)),
        load: parse_loadavg(&text("loadavg")),
        uptime_seconds: text("uptime")
            .split_whitespace()
            .next()
            .and_then(|f| f.parse().ok())
            .unwrap_or(0.0),
    }
}

impl SystemSnapshot {
    // Cores the kernel reports; the machine line is not a core
    pub fn cores(&self) -> usize {
        self.cpus.len().saturating_sub(1).max(1)
    }
}

// ── What the views show ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct Interface {
    pub name: String,
    pub rx_rate: f64,
    pub tx_rate: f64,
    pub rx_total: u64,
    pub tx_total: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Disk {
    pub name: String,
    pub read_rate: f64,
    pub write_rate: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct System {
    pub cpu: f64,
    pub cores: Vec<f64>,
    pub memory: Memory,
    pub load: [f64; 3],
    pub uptime_seconds: f64,
    pub rx_rate: f64,
    pub tx_rate: f64,
    pub interfaces: Vec<Interface>,
    pub disks: Vec<Disk>,
}

// Busy share of the ticks that passed between two looks, as a percent
fn busy_percent(before: CpuTimes, after: CpuTimes) -> f64 {
    let total = after.total.saturating_sub(before.total);
    if total == 0 {
        return 0.0;
    }
    after.busy.saturating_sub(before.busy) as f64 / total as f64 * 100.0
}

// Compare two looks into the rates the views show
// a device that appeared between looks has no rate yet, so it reads 0 this round
pub fn diff(before: &SystemSnapshot, after: &SystemSnapshot) -> System {
    let seconds = after.at.duration_since(before.at).as_secs_f64().max(0.001);
    let per_second = |a: u64, b: u64| b.saturating_sub(a) as f64 / seconds;
    let percents: Vec<f64> = after
        .cpus
        .iter()
        .enumerate()
        .map(|(i, now)| {
            before
                .cpus
                .get(i)
                .map_or(0.0, |was| busy_percent(*was, *now))
        })
        .collect();
    let interfaces: Vec<Interface> = after
        .net
        .iter()
        .map(|(name, rx, tx)| {
            let was = before.net.iter().find(|(n, _, _)| n == name);
            Interface {
                name: name.clone(),
                rx_rate: was.map_or(0.0, |w| per_second(w.1, *rx)),
                tx_rate: was.map_or(0.0, |w| per_second(w.2, *tx)),
                rx_total: *rx,
                tx_total: *tx,
            }
        })
        .collect();
    let disks = after
        .disks
        .iter()
        .map(|(name, read, written)| {
            let was = before.disks.iter().find(|(n, _, _)| n == name);
            Disk {
                name: name.clone(),
                read_rate: was.map_or(0.0, |w| per_second(w.1, *read)),
                write_rate: was.map_or(0.0, |w| per_second(w.2, *written)),
            }
        })
        .collect();
    System {
        cpu: percents.first().copied().unwrap_or(0.0),
        cores: percents.iter().skip(1).copied().collect(),
        memory: after.memory,
        load: after.load,
        uptime_seconds: after.uptime_seconds,
        rx_rate: interfaces.iter().map(|i| i.rx_rate).sum(),
        tx_rate: interfaces.iter().map(|i| i.tx_rate).sum(),
        interfaces,
        disks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const STAT_A: &str = "cpu  100 0 100 700 100 0 0 0 50 0\ncpu0 50 0 50 350 50 0 0 0 0 0\ncpu1 50 0 50 350 50 0 0 0 0 0\nintr 1 2 3\n";
    const STAT_B: &str = "cpu  200 0 200 900 100 0 0 0 90 0\ncpu0 150 0 50 350 50 0 0 0 0 0\ncpu1 50 0 150 450 50 0 0 0 0 0\n";

    fn look(at: Instant, stat: &str, net: Vec<(String, u64, u64)>) -> SystemSnapshot {
        SystemSnapshot {
            at,
            cpus: parse_cpu_times(stat),
            memory: Memory::default(),
            net,
            disks: vec![],
            load: [0.0; 3],
            uptime_seconds: 0.0,
        }
    }

    #[test]
    fn cpu_lines_skip_guest_time_and_count_iowait_as_idle() {
        let cpus = parse_cpu_times(STAT_A);
        assert_eq!(
            cpus.len(),
            3,
            "machine line plus two cores; intr is not a cpu line"
        );
        assert_eq!(
            cpus[0],
            CpuTimes {
                busy: 200,
                total: 1000
            }
        );
    }

    #[test]
    fn busy_percent_comes_from_the_difference() {
        let t0 = Instant::now();
        let a = look(t0, STAT_A, vec![("wlan0".into(), 1000, 0)]);
        let b = look(
            t0 + Duration::from_secs(2),
            STAT_B,
            vec![("wlan0".into(), 5000, 400), ("wg0".into(), 9, 9)],
        );
        let s = diff(&a, &b);
        assert!((s.cpu - 50.0).abs() < 1e-9, "200 busy of 400 ticks");
        assert_eq!(s.cores, vec![100.0, 50.0]);
        assert_eq!(s.interfaces[0].rx_rate, 2000.0);
        assert_eq!(
            s.interfaces[1].rx_rate, 0.0,
            "a new interface has no rate until the next look"
        );
        assert_eq!(s.rx_rate, 2000.0);
        assert_eq!(a.cores(), 2);
    }

    #[test]
    fn meminfo_reads_bytes_and_derives_used() {
        let m = parse_meminfo(
            "MemTotal: 1000 kB\nMemFree: 100 kB\nMemAvailable: 600 kB\nBuffers: 10 kB\nCached: 90 kB\nSwapCached: 5 kB\nSwapTotal: 200 kB\nSwapFree: 150 kB\n",
        );
        assert_eq!(m.total, 1000 * KIB);
        assert_eq!(m.used, 400 * KIB);
        assert_eq!(
            m.cached,
            100 * KIB,
            "SwapCached must not match the Cached prefix"
        );
        assert_eq!(m.swap_used, 50 * KIB);
    }

    #[test]
    fn net_dev_skips_loopback_and_headers() {
        let text = "Inter-|   Receive\n face |bytes packets\n    lo: 5 1 0 0 0 0 0 0 5 1 0 0 0 0 0 0\n wlan0: 700 3 0 0 0 0 0 0 900 4 0 0 0 0 0 0\n";
        assert_eq!(parse_net_dev(text), vec![("wlan0".to_string(), 700, 900)]);
    }

    #[test]
    fn diskstats_keeps_whole_disks_in_bytes() {
        let text = " 259 0 nvme0n1 10 0 8 0 20 0 16 0\n 259 1 nvme0n1p1 10 0 8 0 20 0 16 0\n";
        assert_eq!(
            parse_diskstats(text, &["nvme0n1".to_string()]),
            vec![("nvme0n1".to_string(), 8 * 512, 16 * 512)]
        );
    }

    #[test]
    fn loadavg_reads_three_numbers() {
        assert_eq!(
            parse_loadavg("0.50 1.25 2.00 3/900 1234\n"),
            [0.5, 1.25, 2.0]
        );
        assert_eq!(parse_loadavg(""), [0.0; 3]);
    }
}
