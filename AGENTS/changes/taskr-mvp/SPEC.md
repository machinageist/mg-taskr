<!--
Author: Jeff
Date: 2026-09-18
Description: mg-taskr MVP — a process and system manager with a Rust core, CLI, TUI and shell panel
Notes: Decided with Jeff on 2026-09-18 (geistos cycle 02 interview)
-->

# mg-taskr MVP

A full task manager in the Windows Task Manager / btop sense: what is running, what it
costs, and control over it.

## Decisions (Jeff, 2026-09-18)

- Process/system manager, not a to-do tracker (mg-remindr and mg-planr own tasks).
- Rust core in this repo; surfaces: CLI with `--json` everywhere, ratatui TUI, and a
  Quickshell panel in dotfiles that talks to the CLI.
- Full control of Jeff's own processes and systemd **user** units. System actions go through
  the one-time geist root helper, restricted to `/etc/geist/taskr.conf`: listed **system units**
  may be started/stopped/restarted, and root/other-user processes whose **name** is listed may be
  ended or killed (the process list starts empty).
- Per-process GPU: Intel iGPU from `/proc/<pid>/fdinfo` DRM usage; NVIDIA (470 driver) shown as
  not available. Network: machine and per-interface totals, not per process.
- No history daemon: graphs are in-session only.
- Hotkey CTRL+SHIFT+ESCAPE opens the shell panel (approved 2026-09-18). The CPU and Memory
  pill cards gain an "Open Task Manager" button; their existing clicks do not change.

## Acceptance

- `mg-taskr processes --json` lists every visible process with pid, ppid, name, command, user,
  state, threads, nice, CPU %, memory (RSS), disk read/write rate (own processes), GPU % (Intel),
  and the app it belongs to. Sort by cpu/mem/io/gpu/name/pid; filter by text; `--tree`.
- `apps`, `system` (per-core CPU, memory, swap, per-interface network, disk I/O, load),
  `services --user|--system`, and `startup` (XDG autostart entries, plus Hyprland autostart
  lines listed read-only) each have `--json`.
- Actions: `signal <pid> term|kill|stop|cont`, `renice <pid> <n>`, `service <user|system>
  <start|stop|restart> <unit>`, `startup <enable|disable> <id>`. Own processes and user units act
  directly; anything else goes through the helper and is refused unless allowlisted.
- The TUI covers the same views and actions from the keyboard.
- The shell panel shows Processes, Apps, Services, Startup and Performance, and acts through the CLI.
- CPU % needs two samples; the CLI samples over a short interval (default 500 ms).
- Nothing reads another app's private store; storage is not needed (no state kept).
