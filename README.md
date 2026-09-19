<!--
Author: Jeff
Date: 2026-09-18
Description: mg-taskr — process and system manager for the Geist suite
-->

# mg-taskr

What the machine is running, what it costs, and control over it. A Rust core with a CLI
(`--json` everywhere), a ratatui TUI, and a Quickshell panel in dotfiles that calls the CLI.
It keeps no state and no history, and runs no daemon.

## Read views

```sh
mg-taskr processes [--sort cpu|mem|io|gpu|name|pid] [--filter TEXT] [--tree] [--limit N] [--json]
mg-taskr apps      [--sort …] [--filter TEXT] [--json]      # processes grouped by app
mg-taskr system    [--json]                                 # CPU per core, memory, swap, network, disks, load
mg-taskr services  [--user | --system] [--json]             # running and installed systemd services
mg-taskr startup   [--json]                                 # XDG autostart + Hyprland autostart lines (read-only)
```

Views that show rates sample twice, `--interval` ms apart (default 500).

- **CPU %** `cpu_share` is out of the whole machine (all cores = 100%); `cpu_core` is out of one core, like `top`.
- **Disk rates** are only readable for your own processes; others show `—` / `null`.
- **GPU %** is Intel iGPU busy time from DRM fdinfo. Processes with no GPU client show `—`.
- **Apps**: a systemd `app-*` scope or service names the app. A systemd-run scope takes the
  name of the process that started it. Anything else is its own app.

## Gates

```sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```
