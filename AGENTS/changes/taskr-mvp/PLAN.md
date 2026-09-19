<!--
Author: Jeff
Date: 2026-09-18
Description: mg-taskr MVP slices, each committed on its own with its gates green
-->

# mg-taskr MVP plan

Gates for every slice: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.

1. **Core sampling** — `/proc` readers (stat, status, cmdline, io, fdinfo, cgroup), two-sample
   CPU %, rates, app grouping; system totals. Tests on fixture `/proc` trees.
2. **CLI read views** — processes (sort, filter, tree), apps, system, services, startup; `--json`.
3. **Actions** — signal, renice, user services, startup enable/disable; helper routing for
   system/other-user actions with the allowlist; root helper gains `taskr-service` and
   `taskr-signal` (reviewed, tested, reinstall required).
4. **TUI** — ratatui views and keyboard actions.
5. **Shell panel** (dotfiles) — Task Manager panel, CPU/Memory card buttons, launcher entry,
   CTRL+SHIFT+ESCAPE.
