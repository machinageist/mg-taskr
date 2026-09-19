// Author: Jeff
// Date: 2026-09-18
// Description: Paint the TUI from State — tabs, one table or dashboard, then a key/status line
// Notes: Named ANSI colours only (Cyan, Yellow, Red …), never RGB, so the terminal's theme
//        decides the actual shades and mg-taskr matches whatever Ghostty is wearing.
//        The table scrolls itself: ratatui keeps the selected row in view

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Gauge, Paragraph, Row, Sparkline, Table, TableState, Tabs,
};

use super::state::{State, TABS, Tab};
use crate::services::Scope;
use crate::units;
use crate::views::SortKey;

const ACCENT: Color = Color::Cyan;
const WARN: Color = Color::Yellow;
const BAD: Color = Color::Red;
const DIM: Color = Color::DarkGray;

// Whole screen: tabs on top, the tab's body, one line at the bottom
pub fn draw(frame: &mut Frame, state: &State) {
    let [top, body, bottom] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    draw_tabs(frame, top, state);
    match state.tab {
        Tab::Processes => draw_processes(frame, body, state),
        Tab::Apps => draw_apps(frame, body, state),
        Tab::Performance => draw_performance(frame, body, state),
        Tab::Services => draw_services(frame, body, state),
        Tab::Startup => draw_startup(frame, body, state),
    }
    draw_footer(frame, bottom, state);
}

// Tab names left, a CPU/memory summary right
fn draw_tabs(frame: &mut Frame, area: Rect, state: &State) {
    let [left, right] =
        Layout::horizontal([Constraint::Min(10), Constraint::Length(34)]).areas(area);
    let titles = TABS
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{} {}", i + 1, t.title()));
    let selected = TABS.iter().position(|t| *t == state.tab).unwrap_or(0);
    frame.render_widget(
        Tabs::new(titles)
            .select(selected)
            .highlight_style(Style::new().fg(ACCENT).bold())
            .divider("│"),
        left,
    );
    if let Some(s) = &state.system {
        let summary = format!(
            "CPU {:>5.1}%  MEM {}/{}",
            s.cpu,
            units::bytes(s.memory.used),
            units::bytes(s.memory.total)
        );
        frame.render_widget(Paragraph::new(summary).right_aligned().fg(DIM), right);
    }
}

// Colour a percentage by how loaded it is
fn load_color(percent: f64) -> Color {
    if percent >= 80.0 {
        BAD
    } else if percent >= 40.0 {
        WARN
    } else {
        Color::Reset
    }
}

// Column header, marked when the list is sorted by it
fn header(label: &str, key: Option<SortKey>, sort: SortKey) -> Cell<'static> {
    let marked = key == Some(sort);
    let text = if marked {
        format!("{label}▼")
    } else {
        label.to_string()
    };
    Cell::from(text).style(if marked {
        Style::new().fg(ACCENT).bold()
    } else {
        Style::new().bold()
    })
}

// A table with a cursor row, scrolled so the cursor stays in view
fn render_table(
    frame: &mut Frame,
    area: Rect,
    title: String,
    head: Row,
    rows: Vec<Row>,
    widths: &[Constraint],
    cursor: usize,
) {
    let table = Table::new(rows, widths)
        .header(head)
        .block(
            Block::new()
                .borders(Borders::TOP)
                .title(title)
                .border_style(Style::new().fg(DIM)),
        )
        .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut table_state = TableState::new().with_selected(Some(cursor));
    frame.render_stateful_widget(table, area, &mut table_state);
}

fn draw_processes(frame: &mut Frame, area: Rect, state: &State) {
    let s = state.sort;
    let head = Row::new([
        header("PID", Some(SortKey::Pid), s),
        header("USER", None, s),
        header("CPU%", Some(SortKey::Cpu), s),
        header("MEM", Some(SortKey::Mem), s),
        header("READ", Some(SortKey::Io), s),
        header("WRITE", None, s),
        header("GPU%", Some(SortKey::Gpu), s),
        header("NAME", Some(SortKey::Name), s),
    ]);
    let rows: Vec<(usize, &crate::sample::Process)> = if state.tree {
        state
            .tree_rows
            .iter()
            .map(|r| (r.depth, &r.process))
            .collect()
    } else {
        state.visible_processes().iter().map(|p| (0, p)).collect()
    };
    let rows: Vec<Row> = rows
        .into_iter()
        .map(|(depth, p)| {
            let name = format!(
                "{}{}{}",
                "  ".repeat(depth),
                p.name,
                if p.state == "stopped" {
                    "  (paused)"
                } else {
                    ""
                }
            );
            Row::new([
                Cell::from(p.pid.to_string()),
                Cell::from(p.user.clone()).fg(if p.mine { Color::Reset } else { DIM }),
                Cell::from(format!("{:.1}", p.cpu_share)).fg(load_color(p.cpu_core)),
                Cell::from(units::bytes(p.memory)),
                Cell::from(units::rate(p.read_rate)),
                Cell::from(units::rate(p.write_rate)),
                Cell::from(p.gpu.map_or("—".into(), |g| format!("{g:.0}"))),
                Cell::from(name),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Length(6),
        Constraint::Length(10),
        Constraint::Length(11),
        Constraint::Length(11),
        Constraint::Length(5),
        Constraint::Min(10),
    ];
    let title = format!(
        " {} processes{} ",
        rows.len(),
        if state.tree { " · tree" } else { "" }
    );
    render_table(frame, area, title, head, rows, &widths, state.cursor());
}

fn draw_apps(frame: &mut Frame, area: Rect, state: &State) {
    let s = state.sort;
    let head = Row::new([
        header("APP", Some(SortKey::Name), s),
        header("PROCS", None, s),
        header("CPU%", Some(SortKey::Cpu), s),
        header("MEM", Some(SortKey::Mem), s),
        header("READ", Some(SortKey::Io), s),
        header("WRITE", None, s),
        header("GPU%", Some(SortKey::Gpu), s),
    ]);
    let rows: Vec<Row> = state
        .apps
        .iter()
        .map(|a| {
            Row::new([
                Cell::from(a.name.clone()),
                Cell::from(a.processes.to_string()),
                Cell::from(format!("{:.1}", a.cpu_share)).fg(load_color(a.cpu_core)),
                Cell::from(units::bytes(a.memory)),
                Cell::from(units::rate(a.read_rate)),
                Cell::from(units::rate(a.write_rate)),
                Cell::from(a.gpu.map_or("—".into(), |g| format!("{g:.0}"))),
            ])
        })
        .collect();
    let widths = [
        Constraint::Min(16),
        Constraint::Length(6),
        Constraint::Length(6),
        Constraint::Length(10),
        Constraint::Length(11),
        Constraint::Length(11),
        Constraint::Length(5),
    ];
    render_table(
        frame,
        area,
        format!(" {} apps ", rows.len()),
        head,
        rows,
        &widths,
        state.cursor(),
    );
}

// A titled sparkline over the history the state keeps
fn spark(
    frame: &mut Frame,
    area: Rect,
    title: String,
    data: &[u64],
    max: Option<u64>,
    color: Color,
) {
    let mut line = Sparkline::default()
        .block(
            Block::new()
                .borders(Borders::TOP)
                .title(title)
                .border_style(Style::new().fg(DIM)),
        )
        // the widget draws from the start of the data, so hand it only the newest `width`
        // points — otherwise a long history shows its oldest end
        .data(&data[data.len().saturating_sub(area.width as usize)..])
        .style(Style::new().fg(color));
    if let Some(m) = max {
        line = line.max(m);
    }
    frame.render_widget(line, area);
}

fn draw_performance(frame: &mut Frame, area: Rect, state: &State) {
    let Some(s) = &state.system else {
        frame.render_widget(Paragraph::new("sampling…").fg(DIM), area);
        return;
    };
    let h = &state.history;
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(area);

    // left: CPU graph, then one bar per core
    let core_rows = s.cores.len().div_ceil(2) as u16;
    let [cpu_graph, cores] =
        Layout::vertical([Constraint::Min(5), Constraint::Length(core_rows + 1)]).areas(left);
    let cpu: Vec<u64> = h.cpu.iter().copied().collect();
    spark(
        frame,
        cpu_graph,
        format!(
            " CPU {:.1}%  load {:.2} {:.2} {:.2} ",
            s.cpu, s.load[0], s.load[1], s.load[2]
        ),
        &cpu,
        Some(100),
        ACCENT,
    );
    let lines: Vec<Line> = s
        .cores
        .chunks(2)
        .enumerate()
        .map(|(row, pair)| {
            let spans: Vec<Span> = pair
                .iter()
                .enumerate()
                .flat_map(|(i, pct)| {
                    let n = row * 2 + i;
                    let filled = (pct / 10.0).round() as usize;
                    vec![
                        Span::raw(format!("{n:>3} ")),
                        Span::styled(
                            "█".repeat(filled.min(10)),
                            Style::new().fg(load_color(*pct)),
                        ),
                        Span::raw(format!(
                            "{}{:>4.0}%  ",
                            "·".repeat(10 - filled.min(10)),
                            pct
                        )),
                    ]
                })
                .collect();
            Line::from(spans)
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::new()
                .borders(Borders::TOP)
                .title(" cores ")
                .border_style(Style::new().fg(DIM)),
        ),
        cores,
    );

    // right: memory and swap gauges, network graphs, disks
    let [mem, swap, rx, tx, disks] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Min(3),
        Constraint::Length(s.disks.len() as u16 + 1),
    ])
    .areas(right);
    let m = &s.memory;
    let ratio = |used: u64, total: u64| {
        if total == 0 {
            0.0
        } else {
            (used as f64 / total as f64).clamp(0.0, 1.0)
        }
    };
    frame.render_widget(
        Gauge::default()
            .block(
                Block::new()
                    .borders(Borders::TOP)
                    .title(" memory ")
                    .border_style(Style::new().fg(DIM)),
            )
            .ratio(ratio(m.used, m.total))
            .label(format!(
                "{} / {}  ({} cached)",
                units::bytes(m.used),
                units::bytes(m.total),
                units::bytes(m.cached)
            ))
            .gauge_style(Style::new().fg(ACCENT)),
        mem,
    );
    frame.render_widget(
        Gauge::default()
            .block(
                Block::new()
                    .borders(Borders::TOP)
                    .title(" swap ")
                    .border_style(Style::new().fg(DIM)),
            )
            .ratio(ratio(m.swap_used, m.swap_total))
            .label(format!(
                "{} / {}",
                units::bytes(m.swap_used),
                units::bytes(m.swap_total)
            ))
            .gauge_style(Style::new().fg(WARN)),
        swap,
    );
    let (down, up): (Vec<u64>, Vec<u64>) = (
        h.rx.iter().copied().collect(),
        h.tx.iter().copied().collect(),
    );
    spark(
        frame,
        rx,
        format!(" ↓ {} ", units::rate(Some(s.rx_rate))),
        &down,
        None,
        Color::Green,
    );
    spark(
        frame,
        tx,
        format!(" ↑ {} ", units::rate(Some(s.tx_rate))),
        &up,
        None,
        Color::Magenta,
    );
    let disk_lines: Vec<Line> = s
        .disks
        .iter()
        .map(|d| {
            Line::from(format!(
                "{:<10} read {:>11}  write {:>11}",
                d.name,
                units::rate(Some(d.read_rate)),
                units::rate(Some(d.write_rate))
            ))
        })
        .collect();
    frame.render_widget(
        Paragraph::new(disk_lines).block(
            Block::new()
                .borders(Borders::TOP)
                .title(format!(
                    " disks · up {} ",
                    units::duration(s.uptime_seconds)
                ))
                .border_style(Style::new().fg(DIM)),
        ),
        disks,
    );
}

fn draw_services(frame: &mut Frame, area: Rect, state: &State) {
    let head =
        Row::new(["UNIT", "ACTIVE", "SUB", "STARTUP", "DESCRIPTION"].map(|h| Cell::from(h).bold()));
    let list = state.visible_services();
    let rows: Vec<Row> = list
        .iter()
        .map(|s| {
            let active = match s.active.as_str() {
                "active" => Color::Green,
                "failed" => BAD,
                _ => DIM,
            };
            Row::new([
                Cell::from(s.unit.clone()),
                Cell::from(s.active.clone()).fg(active),
                Cell::from(s.sub.clone()),
                Cell::from(s.startup.clone().unwrap_or_else(|| "—".into())),
                Cell::from(s.description.clone()),
            ])
        })
        .collect();
    let scope = if state.scope == Scope::User {
        "your services"
    } else {
        "system services (via root helper)"
    };
    let widths = [
        Constraint::Percentage(35),
        Constraint::Length(9),
        Constraint::Length(10),
        Constraint::Length(15),
        Constraint::Min(10),
    ];
    render_table(
        frame,
        area,
        format!(" {} {scope} ", rows.len()),
        head,
        rows,
        &widths,
        state.cursor(),
    );
}

fn draw_startup(frame: &mut Frame, area: Rect, state: &State) {
    let Some(found) = &state.startup else {
        frame.render_widget(Paragraph::new("reading…").fg(DIM), area);
        return;
    };
    let [entries, hypr] = Layout::vertical([
        Constraint::Min(4),
        Constraint::Length(found.hyprland.len() as u16 + 1),
    ])
    .areas(area);
    let head = Row::new(
        ["ENTRY", "AT LOGIN", "FROM", "THIS DESKTOP", "COMMAND"].map(|h| Cell::from(h).bold()),
    );
    let rows: Vec<Row> = found
        .autostart
        .iter()
        .map(|e| {
            Row::new([
                Cell::from(e.name.clone()),
                Cell::from(if e.enabled { "on" } else { "off" }).fg(if e.enabled {
                    Color::Green
                } else {
                    DIM
                }),
                Cell::from(e.source.clone()),
                Cell::from(if e.runs_here { "yes" } else { "no" }),
                Cell::from(e.exec.clone()).fg(DIM),
            ])
        })
        .collect();
    let widths = [
        Constraint::Percentage(30),
        Constraint::Length(9),
        Constraint::Length(7),
        Constraint::Length(13),
        Constraint::Min(10),
    ];
    render_table(
        frame,
        entries,
        " XDG autostart ".into(),
        head,
        rows,
        &widths,
        state.cursor(),
    );
    let lines: Vec<Line> = found
        .hyprland
        .iter()
        .map(|l| Line::from(format!("{:>4}  {}", l.line, l.command)).fg(DIM))
        .collect();
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::new()
                .borders(Borders::TOP)
                .title(" Hyprland autostart (read-only) ")
                .border_style(Style::new().fg(DIM)),
        ),
        hypr,
    );
}

// Bottom line, first match wins: question, filter being typed, last result, key hints
fn draw_footer(frame: &mut Frame, area: Rect, state: &State) {
    let line = if let Some(action) = &state.confirm {
        Line::from(vec![
            Span::styled(action.question(), Style::new().fg(WARN).bold()),
            Span::raw("  y = yes, any other key = no"),
        ])
    } else if state.editing_filter {
        Line::from(vec![
            Span::styled("filter: ", Style::new().fg(ACCENT)),
            Span::raw(state.filter.clone()),
            Span::raw("▏  Enter keeps · Esc clears"),
        ])
    } else if let Some((text, is_error)) = &state.message {
        Line::from(Span::styled(
            text.clone(),
            Style::new().fg(if *is_error { BAD } else { Color::Green }),
        ))
    } else {
        let keys = match state.tab {
            Tab::Processes => {
                "e end · K kill · p pause · c continue · + gentler · s sort · t tree · / filter"
            }
            Tab::Apps => "e end app · K kill app · p pause · c continue · s sort · / filter",
            Tab::Performance => "graphs cover the last two minutes",
            Tab::Services => "S start · X stop · R restart · u user/system · / filter",
            Tab::Startup => "space turn on/off at login",
        };
        let filter = if state.filter.is_empty() {
            String::new()
        } else {
            format!("[{}] ", state.filter)
        };
        Line::from(vec![
            Span::styled(filter, Style::new().fg(ACCENT)),
            Span::raw(keys).fg(DIM),
            Span::raw("  · tab/1-5 · q quit").fg(DIM),
        ])
    };
    frame.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::Service;
    use crate::startup::{Entry, HyprLine, Startup};
    use crate::tui::state::tests::{key, loaded};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyCode;

    // Everything the backend drew, as one string to search
    fn screen(state: &State, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| draw(f, state)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn every_tab_draws_at_normal_and_tiny_sizes() {
        let mut s = loaded();
        s.set_services(vec![Service {
            unit: "mpd.service".into(),
            description: "Music".into(),
            load: "loaded".into(),
            active: "active".into(),
            sub: "running".into(),
            startup: Some("enabled".into()),
            scope: Scope::User,
        }]);
        s.set_startup(Startup {
            autostart: vec![Entry {
                id: "nm-applet".into(),
                name: "NetworkManager Applet".into(),
                exec: "nm-applet".into(),
                comment: String::new(),
                source: "system".into(),
                overrides_system: false,
                enabled: true,
                runs_here: true,
                path: "/etc/xdg/autostart/nm-applet.desktop".into(),
            }],
            hyprland: vec![HyprLine {
                file: "a.lua".into(),
                line: 8,
                command: "hyprpaper".into(),
            }],
        });
        let expect = [
            ("firefox", '1'),
            ("ghostty", '2'),
            ("load 0.50", '3'),
            ("mpd.service", '4'),
            ("hyprpaper", '5'),
        ];
        for (text, tab) in expect {
            s.key(key(KeyCode::Char(tab)));
            assert!(screen(&s, 120, 30).contains(text), "tab {tab} shows {text}");
            // a cramped terminal must not panic
            screen(&s, 20, 5);
        }
    }

    #[test]
    fn a_long_history_graphs_its_newest_end() {
        let mut terminal = Terminal::new(TestBackend::new(10, 3)).unwrap();
        // 20 old full-height points, then 10 new empty ones: a 10-wide graph must be empty
        let data: Vec<u64> = std::iter::repeat_n(100, 20)
            .chain(std::iter::repeat_n(0, 10))
            .collect();
        terminal
            .draw(|f| spark(f, f.area(), String::new(), &data, Some(100), ACCENT))
            .unwrap();
        let drawn: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(!drawn.contains('█'), "{drawn:?}");
    }

    #[test]
    fn the_footer_shows_the_question_while_one_is_open() {
        let mut s = loaded();
        s.key(key(KeyCode::Char('K')));
        assert!(screen(&s, 120, 10).contains("Force-kill firefox (20)?"));
    }
}
