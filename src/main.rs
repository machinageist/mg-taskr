// Author: Jeff
// Date: 2026-09-18
// Description: mg-taskr command line — processes, apps, system, services, startup
// Notes: Every view has --json for the shell panel and scripts; without it, a plain table.
//        Rates need two looks, so sampling views wait --interval ms (default 500) between them

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use serde::Serialize;

use mg_taskr::services::{self, Scope};
use mg_taskr::views::{self, SortKey};
use mg_taskr::{os, procfs, sample, startup, system, units};

const PROC_ROOT: &str = "/proc";
const SYS_ROOT: &str = "/sys";
const PASSWD: &str = "/etc/passwd";
const DEFAULT_INTERVAL_MS: u64 = 500;

#[derive(Parser)]
#[command(
    name = "mg-taskr",
    version,
    about = "Process and system manager for the Geist suite"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct Sampling {
    /// Milliseconds between the two looks that rates are measured over
    #[arg(long, default_value_t = DEFAULT_INTERVAL_MS)]
    interval: u64,
    /// Print JSON instead of a table
    #[arg(long)]
    json: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Every visible process with its CPU, memory, disk and GPU use
    Processes {
        #[arg(long, value_enum, default_value_t)]
        sort: SortKey,
        /// Keep rows whose name, command, user, app or pid match
        #[arg(long, default_value = "")]
        filter: String,
        /// Children under their parents
        #[arg(long)]
        tree: bool,
        /// Show at most this many rows
        #[arg(long)]
        limit: Option<usize>,
        #[command(flatten)]
        sampling: Sampling,
    },
    /// Processes grouped by the app they belong to
    Apps {
        #[arg(long, value_enum, default_value_t)]
        sort: SortKey,
        #[arg(long, default_value = "")]
        filter: String,
        #[command(flatten)]
        sampling: Sampling,
    },
    /// Machine totals: CPU per core, memory, swap, network, disks, load
    System {
        #[command(flatten)]
        sampling: Sampling,
    },
    /// systemd services, running or not
    Services {
        /// The machine's services instead of yours
        #[arg(long, conflicts_with = "user")]
        system: bool,
        /// Your services (the default)
        #[arg(long)]
        user: bool,
        #[arg(long)]
        json: bool,
    },
    /// What starts at login
    Startup {
        #[arg(long)]
        json: bool,
    },
}

fn main() -> Result<()> {
    os::exit_on_closed_pipe();
    match Cli::parse().command {
        Command::Processes {
            sort,
            filter,
            tree,
            limit,
            sampling,
        } => {
            let (rows, _) = sample_all(sampling.interval);
            let limit = limit.unwrap_or(usize::MAX);
            if tree {
                let mut rows = views::tree(rows, sort, &filter);
                rows.truncate(limit);
                print_or(sampling.json, &rows, || {
                    print_processes(rows.iter().map(|r| (r.depth, &r.process)))
                })
            } else {
                let mut rows = views::filter(rows, &filter);
                views::sort(&mut rows, sort);
                rows.truncate(limit);
                print_or(sampling.json, &rows, || {
                    print_processes(rows.iter().map(|p| (0, p)))
                })
            }
        }
        Command::Apps {
            sort,
            filter,
            sampling,
        } => {
            let (rows, _) = sample_all(sampling.interval);
            let apps = views::apps(&views::filter(rows, &filter), sort);
            print_or(sampling.json, &apps, || print_apps(&apps))
        }
        Command::System { sampling } => {
            let (_, totals) = sample_all(sampling.interval);
            print_or(sampling.json, &totals, || print_system(&totals))
        }
        Command::Services { system, json, .. } => {
            let scope = if system { Scope::System } else { Scope::User };
            let list = services::list(scope)?;
            print_or(json, &list, || print_services(&list))
        }
        Command::Startup { json } => {
            let found = startup::list();
            print_or(json, &found, || print_startup(&found))
        }
    }
}

// Two looks at every process and the machine, `interval_ms` apart → rows and totals
fn sample_all(interval_ms: u64) -> (Vec<sample::Process>, system::System) {
    let (proc_root, sys_root) = (Path::new(PROC_ROOT), Path::new(SYS_ROOT));
    let users: HashMap<u32, String> = std::fs::read_to_string(PASSWD)
        .map(|t| procfs::parse_passwd(&t))
        .unwrap_or_default();
    let (procs_before, system_before) = (
        sample::snapshot(proc_root),
        system::snapshot(proc_root, sys_root),
    );
    std::thread::sleep(Duration::from_millis(interval_ms.max(1)));
    let (procs_after, system_after) = (
        sample::snapshot(proc_root),
        system::snapshot(proc_root, sys_root),
    );
    let cores = system_after.cores() as f64;
    let rows = sample::diff(
        &procs_before,
        &procs_after,
        os::ticks_per_second(),
        cores,
        &users,
        os::my_uid(),
    );
    (rows, system::diff(&system_before, &system_after))
}

// JSON when asked, otherwise the table printer
fn print_or<T: Serialize>(json: bool, value: &T, table: impl FnOnce()) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string(value)?);
    } else {
        table();
    }
    Ok(())
}

// Cut text to a width, marking the cut
fn fit(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(width.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

// ── Tables ───────────────────────────────────────────────────────────────

fn print_processes<'a>(rows: impl Iterator<Item = (usize, &'a sample::Process)>) {
    println!(
        "{:>7} {:<10} {:>6} {:>10} {:>11} {:>11} {:>5}  NAME",
        "PID", "USER", "CPU%", "MEM", "READ", "WRITE", "GPU%"
    );
    for (depth, p) in rows {
        let gpu = p.gpu.map_or("—".into(), |g| format!("{g:.0}"));
        let name = format!("{}{}", "  ".repeat(depth), p.name);
        println!(
            "{:>7} {:<10} {:>6.1} {:>10} {:>11} {:>11} {:>5}  {}",
            p.pid,
            fit(&p.user, 10),
            p.cpu_share,
            units::bytes(p.memory),
            units::rate(p.read_rate),
            units::rate(p.write_rate),
            gpu,
            name
        );
    }
}

fn print_apps(apps: &[views::App]) {
    println!(
        "{:>6} {:>10} {:>11} {:>11} {:>5} {:>5}  APP",
        "CPU%", "MEM", "READ", "WRITE", "GPU%", "PROCS"
    );
    for a in apps {
        let gpu = a.gpu.map_or("—".into(), |g| format!("{g:.0}"));
        println!(
            "{:>6.1} {:>10} {:>11} {:>11} {:>5} {:>5}  {}",
            a.cpu_share,
            units::bytes(a.memory),
            units::rate(a.read_rate),
            units::rate(a.write_rate),
            gpu,
            a.processes,
            a.name
        );
    }
}

fn print_system(s: &system::System) {
    let m = &s.memory;
    println!(
        "CPU     {:.1}%  load {:.2} {:.2} {:.2}  up {}",
        s.cpu,
        s.load[0],
        s.load[1],
        s.load[2],
        units::duration(s.uptime_seconds)
    );
    let cores: Vec<String> = s.cores.iter().map(|c| format!("{c:.0}")).collect();
    println!("Cores   {}", cores.join(" "));
    println!(
        "Memory  {} of {} used, {} cached",
        units::bytes(m.used),
        units::bytes(m.total),
        units::bytes(m.cached)
    );
    println!(
        "Swap    {} of {}",
        units::bytes(m.swap_used),
        units::bytes(m.swap_total)
    );
    println!(
        "Network ↓ {}  ↑ {}",
        units::rate(Some(s.rx_rate)),
        units::rate(Some(s.tx_rate))
    );
    for i in &s.interfaces {
        println!(
            "  {:<12} ↓ {:>11}  ↑ {:>11}",
            i.name,
            units::rate(Some(i.rx_rate)),
            units::rate(Some(i.tx_rate))
        );
    }
    println!("Disks");
    for d in &s.disks {
        println!(
            "  {:<12} read {:>11}  write {:>11}",
            d.name,
            units::rate(Some(d.read_rate)),
            units::rate(Some(d.write_rate))
        );
    }
}

fn print_services(list: &[services::Service]) {
    println!(
        "{:<44} {:<9} {:<10} {:<15}  DESCRIPTION",
        "UNIT", "ACTIVE", "SUB", "STARTUP"
    );
    for s in list {
        println!(
            "{:<44} {:<9} {:<10} {:<15}  {}",
            fit(&s.unit, 44),
            s.active,
            s.sub,
            s.startup.as_deref().unwrap_or("—"),
            s.description
        );
    }
}

fn print_startup(found: &startup::Startup) {
    println!(
        "{:<24} {:<7} {:<8} {:<5}  EXEC",
        "AUTOSTART", "SOURCE", "ENABLED", "HERE"
    );
    for e in &found.autostart {
        let yes = |b: bool| if b { "yes" } else { "no" };
        println!(
            "{:<24} {:<7} {:<8} {:<5}  {}",
            fit(&e.id, 24),
            e.source,
            yes(e.enabled),
            yes(e.runs_here),
            e.exec
        );
    }
    println!();
    println!("HYPRLAND (read-only)");
    for l in &found.hyprland {
        println!("  {}:{:<4} {}", l.file.display(), l.line, l.command);
    }
}
