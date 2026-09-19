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
mg-taskr watch     [--interval MS]                          # NDJSON: {system, processes, apps} per look, for the shell panel
```

Views that show rates sample twice, `--interval` ms apart (default 500).

## Actions

```sh
mg-taskr signal  <pid>... term|kill|stop|cont               # several pids: every one is tried
mg-taskr renice  <pid> <-20..19>                            # own processes; slower only
mg-taskr service user|system start|stop|restart <unit>
mg-taskr startup enable|disable <id>                        # writes ~/.config/autostart/<id>.desktop
```

Your own processes and user units are acted on directly. Root or other-user processes and
system units go through the geist root helper (`~/dotfiles/system/`), which refuses anything
not listed in the root-owned `/etc/geist/taskr.conf` (empty at install). Renice can only make a
process gentler; making one faster needs root and is not offered. `--json` prints
`{"ok":true,"message":…}` or `{"ok":false,"error":…}` (exit 1).

- **CPU %** `cpu_share` is out of the whole machine (all cores = 100%); `cpu_core` is out of one core, like `top`.
- **Disk rates** are only readable for your own processes; others show `—` / `null`.
- **GPU %** is Intel iGPU busy time from DRM fdinfo. Processes with no GPU client show `—`.
- **Apps**: a systemd `app-*` scope or service names the app. A systemd-run scope takes the
  name of the process that started it. Kernel threads are one app. Anything else is its own app.

## TUI

```sh
mg-taskr tui
```

Tabs: `1` Processes, `2` Apps, `3` Performance, `4` Services, `5` Startup (`Tab`/`Shift+Tab` cycle).
Move with arrows or `j`/`k`, `PgUp`/`PgDn`, `g`/`G`. `/` filters, `Esc` clears it (a second `Esc`
or `q` quits).

| Tab | Keys |
|---|---|
| Processes | `e`/`Delete` end, `K` kill (both ask y/n), `p` pause, `c` continue, `+` gentler, `s` sort, `t` tree |
| Apps | the same signals, sent to every process of the app |
| Services | `S` start, `X` stop, `R` restart (ask y/n), `u` your services ↔ system services |
| Startup | `space` turn an autostart entry on or off at login |

The graphs cover the last two minutes of this session; nothing is kept after you quit.
Colours are the terminal's named ANSI colours, so the TUI follows the terminal theme.

## Gates

```sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```
