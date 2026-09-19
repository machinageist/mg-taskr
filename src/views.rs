// Author: Jeff
// Date: 2026-09-18
// Description: Shape sampled processes for display — sort, filter, tree, and group by app
// Notes: Pure functions over sample::Process rows so the CLI, TUI and tests share them.
//        Big numbers sort first (CPU, memory, disk, GPU); name and pid sort ascending.
//        A filtered tree keeps each match's ancestors, so a hit never floats without context

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::sample::Process;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum SortKey {
    #[default]
    Cpu,
    Mem,
    Io,
    Gpu,
    Name,
    Pid,
}

// Disk traffic both ways; a process we cannot read counts as none
fn io_rate(p: &Process) -> f64 {
    p.read_rate.unwrap_or(0.0) + p.write_rate.unwrap_or(0.0)
}

// Order two processes by the chosen key, pid as the tie-breaker so the list never jitters
fn compare(key: SortKey, a: &Process, b: &Process) -> Ordering {
    let biggest_first = |x: f64, y: f64| y.partial_cmp(&x).unwrap_or(Ordering::Equal);
    let by_key = match key {
        SortKey::Cpu => biggest_first(a.cpu_share, b.cpu_share),
        SortKey::Mem => b.memory.cmp(&a.memory),
        SortKey::Io => biggest_first(io_rate(a), io_rate(b)),
        SortKey::Gpu => biggest_first(a.gpu.unwrap_or(0.0), b.gpu.unwrap_or(0.0)),
        SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        SortKey::Pid => Ordering::Equal,
    };
    by_key.then(a.pid.cmp(&b.pid))
}

// Sort rows in place by the chosen key
pub fn sort(rows: &mut [Process], key: SortKey) {
    rows.sort_by(|a, b| compare(key, a, b));
}

// Does this row mention the text anywhere a person would look — name, command, user, app, pid
pub fn matches(p: &Process, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let needle = needle.to_lowercase();
    p.pid.to_string() == needle
        || [&p.name, &p.command, &p.user, &p.app]
            .iter()
            .any(|field| field.to_lowercase().contains(&needle))
}

// Keep only the rows that match
pub fn filter(rows: Vec<Process>, needle: &str) -> Vec<Process> {
    rows.into_iter().filter(|p| matches(p, needle)).collect()
}

// ── Tree ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct TreeRow {
    pub depth: usize,
    #[serde(flatten)]
    pub process: Process,
}

// Parent-before-children order with a depth for indenting
// siblings follow the sort key; a process whose parent we cannot see becomes a root
pub fn tree(rows: Vec<Process>, key: SortKey, needle: &str) -> Vec<TreeRow> {
    let by_pid: HashMap<u32, &Process> = rows.iter().map(|p| (p.pid, p)).collect();

    // which rows stay: the matches, plus every ancestor of a match
    let mut keep: HashSet<u32> = HashSet::new();
    for p in rows.iter().filter(|p| matches(p, needle)) {
        let mut pid = p.pid;
        // the walk stops at a pid already kept, so a loop in bad data cannot spin forever
        while keep.insert(pid) {
            match by_pid.get(&pid) {
                Some(parent) if by_pid.contains_key(&parent.ppid) => pid = parent.ppid,
                _ => break,
            }
        }
    }

    let mut children: HashMap<u32, Vec<&Process>> = HashMap::new();
    let mut roots: Vec<&Process> = Vec::new();
    for p in rows.iter().filter(|p| keep.contains(&p.pid)) {
        if p.ppid != p.pid && keep.contains(&p.ppid) {
            children.entry(p.ppid).or_default().push(p);
        } else {
            roots.push(p);
        }
    }
    for list in children.values_mut() {
        list.sort_by(|a, b| compare(key, a, b));
    }
    roots.sort_by(|a, b| compare(key, a, b));

    // depth-first walk with an explicit stack — deep trees cannot overflow it
    let mut out = Vec::with_capacity(keep.len());
    let mut stack: Vec<(&Process, usize)> = roots.into_iter().rev().map(|p| (p, 0)).collect();
    while let Some((p, depth)) = stack.pop() {
        out.push(TreeRow {
            depth,
            process: p.clone(),
        });
        if let Some(kids) = children.get(&p.pid) {
            stack.extend(kids.iter().rev().map(|k| (*k, depth + 1)));
        }
    }
    out
}

// ── Apps ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct App {
    pub name: String,
    pub processes: usize,
    pub pids: Vec<u32>,
    pub cpu_share: f64,
    pub cpu_core: f64,
    pub memory: u64,
    pub read_rate: Option<f64>,
    pub write_rate: Option<f64>,
    pub gpu: Option<f64>,
    pub mine: bool,
}

// Add an optional rate into a running total; unknown stays unknown only if every part is
fn add(total: Option<f64>, part: Option<f64>) -> Option<f64> {
    match (total, part) {
        (None, None) => None,
        (t, p) => Some(t.unwrap_or(0.0) + p.unwrap_or(0.0)),
    }
}

// One row per app, summing its processes; sorted like processes
pub fn apps(rows: &[Process], key: SortKey) -> Vec<App> {
    let mut groups: HashMap<&str, App> = HashMap::new();
    for p in rows {
        let app = groups.entry(p.app.as_str()).or_insert_with(|| App {
            name: p.app.clone(),
            processes: 0,
            pids: Vec::new(),
            cpu_share: 0.0,
            cpu_core: 0.0,
            memory: 0,
            read_rate: None,
            write_rate: None,
            gpu: None,
            mine: true,
        });
        app.processes += 1;
        app.pids.push(p.pid);
        app.cpu_share += p.cpu_share;
        app.cpu_core += p.cpu_core;
        app.memory += p.memory;
        app.read_rate = add(app.read_rate, p.read_rate);
        app.write_rate = add(app.write_rate, p.write_rate);
        app.gpu = add(app.gpu, p.gpu);
        // one foreign process makes the whole app "not only mine" — actions may need the helper
        app.mine &= p.mine;
    }
    let mut list: Vec<App> = groups.into_values().collect();
    for app in &mut list {
        app.pids.sort_unstable();
    }
    let biggest_first = |x: f64, y: f64| y.partial_cmp(&x).unwrap_or(Ordering::Equal);
    list.sort_by(|a, b| {
        let by_key = match key {
            SortKey::Cpu => biggest_first(a.cpu_share, b.cpu_share),
            SortKey::Mem => b.memory.cmp(&a.memory),
            SortKey::Io => biggest_first(
                a.read_rate.unwrap_or(0.0) + a.write_rate.unwrap_or(0.0),
                b.read_rate.unwrap_or(0.0) + b.write_rate.unwrap_or(0.0),
            ),
            SortKey::Gpu => biggest_first(a.gpu.unwrap_or(0.0), b.gpu.unwrap_or(0.0)),
            SortKey::Name | SortKey::Pid => Ordering::Equal,
        };
        by_key.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pid: u32, ppid: u32, name: &str, cpu: f64, memory: u64, app: &str) -> Process {
        Process {
            pid,
            ppid,
            name: name.into(),
            command: format!("/usr/bin/{name}"),
            user: "mgeist".into(),
            uid: Some(1000),
            state: "sleeping".into(),
            threads: 1,
            nice: 0,
            cpu_share: cpu,
            cpu_core: cpu,
            memory,
            read_rate: None,
            write_rate: None,
            gpu: None,
            app: app.into(),
            mine: true,
        }
    }

    fn sample_rows() -> Vec<Process> {
        vec![
            row(1, 0, "systemd", 0.1, 10, "init"),
            row(20, 1, "firefox", 30.0, 500, "firefox"),
            row(21, 20, "Web Content", 12.0, 300, "firefox"),
            row(22, 20, "Isolated Web", 40.0, 200, "firefox"),
            row(30, 1, "ghostty", 1.0, 80, "ghostty"),
            row(31, 30, "zsh", 0.0, 5, "ghostty"),
        ]
    }

    #[test]
    fn sorting_puts_big_numbers_first_and_names_in_order() {
        let mut rows = sample_rows();
        sort(&mut rows, SortKey::Cpu);
        assert_eq!(
            rows.iter().map(|p| p.pid).collect::<Vec<_>>(),
            [22, 20, 21, 30, 1, 31]
        );
        sort(&mut rows, SortKey::Name);
        assert_eq!(rows[0].name, "firefox", "case does not decide the order");
    }

    #[test]
    fn filter_looks_at_name_command_app_and_exact_pid() {
        let rows = sample_rows();
        assert_eq!(filter(rows.clone(), "WEB").len(), 2);
        assert_eq!(
            filter(rows.clone(), "ghostty").len(),
            2,
            "zsh matches through its app"
        );
        assert_eq!(
            filter(rows.clone(), "2").len(),
            0,
            "a pid must match whole, not by digit"
        );
        assert_eq!(filter(rows, "21")[0].pid, 21);
    }

    #[test]
    fn tree_nests_children_under_parents_in_sort_order() {
        let t = tree(sample_rows(), SortKey::Cpu, "");
        let shape: Vec<(u32, usize)> = t.iter().map(|r| (r.process.pid, r.depth)).collect();
        assert_eq!(shape, [(1, 0), (20, 1), (22, 2), (21, 2), (30, 1), (31, 2)]);
    }

    #[test]
    fn a_filtered_tree_keeps_the_ancestors_of_each_match() {
        let t = tree(sample_rows(), SortKey::Pid, "zsh");
        let pids: Vec<u32> = t.iter().map(|r| r.process.pid).collect();
        assert_eq!(pids, [1, 30, 31]);
    }

    #[test]
    fn apps_sum_their_processes() {
        let mut rows = sample_rows();
        rows[2].read_rate = Some(100.0);
        rows[3].mine = false;
        let list = apps(&rows, SortKey::Cpu);
        assert_eq!(list[0].name, "firefox");
        assert_eq!(list[0].processes, 3);
        assert_eq!(list[0].memory, 1000);
        assert!((list[0].cpu_share - 82.0).abs() < 1e-9);
        assert_eq!(list[0].read_rate, Some(100.0));
        assert_eq!(
            list[0].write_rate, None,
            "no process could say, so the app cannot either"
        );
        assert!(!list[0].mine);
        assert_eq!(list[0].pids, [20, 21, 22]);
    }
}
