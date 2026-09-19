// Author: Jeff
// Date: 2026-09-18
// Description: Point the readers at a fake /proc and /sys and check what comes out
// Notes: Builds the tree under the system temp dir, one folder per test, removed at the end

use std::fs;
use std::path::{Path, PathBuf};

use mg_taskr::{sample, system};

// A fresh empty folder for one test
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mg-taskr-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

// Write one fixture file, making its folders
fn put(root: &Path, rel: &str, body: &[u8]) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().expect("has parent")).expect("mkdir");
    fs::write(path, body).expect("write");
}

#[test]
fn snapshot_reads_every_process_and_skips_half_gone_ones() {
    let root = scratch("procs");
    put(
        &root,
        "42/stat",
        b"42 (firefox) S 1 42 42 0 -1 0 0 0 0 0 30 10 0 0 20 0 80 0 5000 0 0",
    );
    put(
        &root,
        "42/status",
        b"Name:\tfirefox\nUid:\t1000\t1000\t1000\t1000\nVmRSS:\t  2048 kB\n",
    );
    put(
        &root,
        "42/cmdline",
        b"/usr/lib/firefox/firefox\0--new-window\0",
    );
    put(&root, "42/cgroup", b"0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-hyprland-firefox-1234.scope\n");
    put(
        &root,
        "42/io",
        b"rchar: 1\nwchar: 2\nread_bytes: 4096\nwrite_bytes: 8192\n",
    );
    // exited between listing and reading: a folder with no stat
    fs::create_dir_all(root.join("43")).expect("mkdir");
    // not a pid
    put(&root, "meminfo", b"MemTotal: 1 kB\n");

    let snap = sample::snapshot(&root);
    assert_eq!(snap.procs.len(), 1);
    let fx = &snap.procs[&42];
    assert_eq!(fx.stat.comm, "firefox");
    assert_eq!(fx.uid, Some(1000));
    assert_eq!(fx.rss, Some(2048 * 1024));
    assert_eq!(fx.cmdline, "/usr/lib/firefox/firefox --new-window");
    assert_eq!(fx.app.as_deref(), Some("firefox"));
    assert_eq!(fx.io, Some((4096, 8192)));
    fs::remove_dir_all(root).ok();
}

#[test]
fn system_snapshot_counts_whole_disks_only() {
    let root = scratch("system");
    let proc_root = root.join("proc");
    let sys_root = root.join("sys");
    put(
        &proc_root,
        "stat",
        b"cpu  1 0 1 8 0 0 0 0 0 0\ncpu0 1 0 1 8 0 0 0 0 0 0\n",
    );
    put(
        &proc_root,
        "meminfo",
        b"MemTotal: 100 kB\nMemAvailable: 25 kB\n",
    );
    put(
        &proc_root,
        "net/dev",
        b"h\nh\n eth0: 10 0 0 0 0 0 0 0 20 0 0 0 0 0 0 0\n",
    );
    put(
        &proc_root,
        "diskstats",
        b" 8 0 sda 1 0 2 0 1 0 4 0\n 8 1 sda1 1 0 2 0 1 0 4 0\n 7 0 loop0 1 0 2 0 1 0 4 0\n",
    );
    put(&proc_root, "loadavg", b"0.10 0.20 0.30 1/100 5\n");
    put(&proc_root, "uptime", b"123.45 99.00\n");
    for disk in ["sda", "loop0", "zram0"] {
        fs::create_dir_all(sys_root.join("block").join(disk)).expect("mkdir");
    }

    let snap = system::snapshot(&proc_root, &sys_root);
    assert_eq!(system::whole_disks(&sys_root), vec!["sda".to_string()]);
    assert_eq!(snap.disks, vec![("sda".to_string(), 1024, 2048)]);
    assert_eq!(snap.memory.used, 75 * 1024);
    assert_eq!(snap.net, vec![("eth0".to_string(), 10, 20)]);
    assert_eq!(snap.cores(), 1);
    assert_eq!(snap.uptime_seconds, 123.45);
    fs::remove_dir_all(root).ok();
}
