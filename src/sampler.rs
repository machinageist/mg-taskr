// Author: Jeff
// Date: 2026-09-18
// Description: Keeps the last look at the machine so each new look yields rates at once
// Notes: The CLI takes one look, waits, takes another. The TUI and the shell's live view call
//        `tick` on a timer instead: every call diffs against the previous call, no extra wait

use std::collections::HashMap;
use std::path::PathBuf;

use crate::{os, procfs, sample, system};

const PASSWD: &str = "/etc/passwd";

pub struct Sampler {
    proc_root: PathBuf,
    sys_root: PathBuf,
    users: HashMap<u32, String>,
    me: u32,
    ticks_per_second: f64,
    procs: sample::Snapshot,
    machine: system::SystemSnapshot,
}

impl Sampler {
    // Take the first look now; the first tick measures from here
    pub fn new(proc_root: impl Into<PathBuf>, sys_root: impl Into<PathBuf>) -> Self {
        let (proc_root, sys_root) = (proc_root.into(), sys_root.into());
        let users = std::fs::read_to_string(PASSWD)
            .map(|t| procfs::parse_passwd(&t))
            .unwrap_or_default();
        let procs = sample::snapshot(&proc_root);
        let machine = system::snapshot(&proc_root, &sys_root);
        Sampler {
            proc_root,
            sys_root,
            users,
            me: os::my_uid(),
            ticks_per_second: os::ticks_per_second(),
            procs,
            machine,
        }
    }

    // Look again and return what changed since the last look
    pub fn tick(&mut self) -> (Vec<sample::Process>, system::System) {
        let procs = sample::snapshot(&self.proc_root);
        let machine = system::snapshot(&self.proc_root, &self.sys_root);
        let cores = machine.cores() as f64;
        let rows = sample::diff(
            &self.procs,
            &procs,
            self.ticks_per_second,
            cores,
            &self.users,
            self.me,
        );
        let totals = system::diff(&self.machine, &machine);
        (self.procs, self.machine) = (procs, machine);
        (rows, totals)
    }
}
