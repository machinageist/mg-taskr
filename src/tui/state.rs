// Author: Jeff
// Date: 2026-09-18
// Description: Everything the TUI knows and how each key changes it — no terminal involved
// Notes: `key` turns a keypress into a change of state plus an Effect the loop carries out
//        (quit, run an action, reload a list). Keeping it pure means tests drive it with plain
//        key events. Anything that ends, stops or pauses something asks y/n first — pausing the
//        compositor or this terminal would freeze the very screen you need to resume it.
//        Continue, gentler and startup toggles are harmless or undoable, so they act at once.
//        Selection follows the thing, not the row number: after a refresh the cursor stays on
//        the same pid / app / unit / entry even when the list reorders

use std::collections::VecDeque;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::actions::{ServiceVerb, Signal};
use crate::sample::Process;
use crate::services::{Scope, Service};
use crate::startup::Startup;
use crate::system::System;
use crate::views::{self, App, SortKey, TreeRow};

// how many ticks of history the graphs keep — two minutes at one look per second
pub const HISTORY: usize = 120;
const PAGE: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Processes,
    Apps,
    Performance,
    Services,
    Startup,
}

pub const TABS: [Tab; 5] = [
    Tab::Processes,
    Tab::Apps,
    Tab::Performance,
    Tab::Services,
    Tab::Startup,
];

impl Tab {
    pub fn title(self) -> &'static str {
        match self {
            Tab::Processes => "Processes",
            Tab::Apps => "Apps",
            Tab::Performance => "Performance",
            Tab::Services => "Services",
            Tab::Startup => "Startup",
        }
    }

    fn index(self) -> usize {
        TABS.iter().position(|t| *t == self).unwrap_or(0)
    }
}

// Something that needs doing outside the state — the loop runs it
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Signal {
        pids: Vec<u32>,
        signal: Signal,
        label: String,
    },
    Renice {
        pid: u32,
        nice: i32,
        label: String,
    },
    Service {
        scope: Scope,
        verb: ServiceVerb,
        unit: String,
    },
    Startup {
        id: String,
        enable: bool,
    },
}

impl Action {
    // The y/n question shown before a risky action
    pub fn question(&self) -> String {
        match self {
            Action::Signal {
                signal: Signal::Term,
                label,
                ..
            } => format!("End {label}?"),
            Action::Signal {
                signal: Signal::Kill,
                label,
                ..
            } => format!("Force-kill {label}? Unsaved work is lost."),
            Action::Signal {
                signal: Signal::Stop,
                label,
                ..
            } => format!("Pause {label}? It stays frozen until continued."),
            Action::Signal { signal, label, .. } => format!("{} {label}?", signal.word()),
            Action::Service { verb, unit, .. } => format!("{} {unit}?", verb.word()),
            Action::Renice { label, nice, .. } => format!("Set {label} to nice {nice}?"),
            Action::Startup { id, enable } => format!(
                "{} {id} at login?",
                if *enable { "Start" } else { "Stop starting" }
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    None,
    Quit,
    Run(Action),
    LoadServices,
    LoadStartup,
}

#[derive(Default)]
pub struct History {
    pub cpu: VecDeque<u64>,
    pub memory: VecDeque<u64>,
    pub rx: VecDeque<u64>,
    pub tx: VecDeque<u64>,
}

impl History {
    // Add one tick, dropping the oldest past the limit
    fn push(&mut self, s: &System) {
        // no memory total (a broken /proc) reads as 0% rather than dividing by zero
        let used = (s.memory.used * 100)
            .checked_div(s.memory.total)
            .unwrap_or(0);
        for (series, value) in [
            (&mut self.cpu, s.cpu as u64),
            (&mut self.memory, used),
            (&mut self.rx, s.rx_rate as u64),
            (&mut self.tx, s.tx_rate as u64),
        ] {
            if series.len() == HISTORY {
                series.pop_front();
            }
            series.push_back(value);
        }
    }
}

pub struct State {
    pub tab: Tab,
    pub sort: SortKey,
    pub tree: bool,
    pub filter: String,
    pub editing_filter: bool,
    pub scope: Scope,
    // one cursor per tab
    pub cursor: [usize; 5],
    pub confirm: Option<Action>,
    // last result line: (text, is an error)
    pub message: Option<(String, bool)>,
    // every process from the last sample, then the sorted, filtered list the table shows
    pub processes: Vec<Process>,
    processes_view: Vec<Process>,
    pub tree_rows: Vec<TreeRow>,
    pub apps: Vec<App>,
    pub system: Option<System>,
    pub history: History,
    pub services: Vec<Service>,
    pub startup: Option<Startup>,
}

impl Default for State {
    fn default() -> Self {
        State {
            tab: Tab::Processes,
            sort: SortKey::Cpu,
            tree: false,
            filter: String::new(),
            editing_filter: false,
            scope: Scope::User,
            cursor: [0; 5],
            confirm: None,
            message: None,
            processes: Vec::new(),
            processes_view: Vec::new(),
            tree_rows: Vec::new(),
            apps: Vec::new(),
            system: None,
            history: History::default(),
            services: Vec::new(),
            startup: None,
        }
    }
}

// Next sort key in the cycle the `s` key walks
fn next_sort(key: SortKey) -> SortKey {
    match key {
        SortKey::Cpu => SortKey::Mem,
        SortKey::Mem => SortKey::Io,
        SortKey::Io => SortKey::Gpu,
        SortKey::Gpu => SortKey::Name,
        SortKey::Name => SortKey::Pid,
        SortKey::Pid => SortKey::Cpu,
    }
}

impl State {
    // ── Data arriving ────────────────────────────────────────────────────

    // A fresh sample: rebuild the lists, keep each cursor on the same thing
    pub fn set_sample(&mut self, rows: Vec<Process>, system: System) {
        let (pid, app) = (
            self.selected_pid(),
            self.selected_app().map(|a| a.name.clone()),
        );
        self.history.push(&system);
        self.system = Some(system);
        self.processes = rows;
        self.rebuild();
        if let Some(pid) = pid {
            let at = if self.tree {
                self.tree_rows.iter().position(|r| r.process.pid == pid)
            } else {
                self.visible_processes().iter().position(|p| p.pid == pid)
            };
            self.follow(Tab::Processes, at);
        }
        if let Some(name) = app {
            let at = self.apps.iter().position(|a| a.name == name);
            self.follow(Tab::Apps, at);
        }
        self.clamp_all();
    }

    pub fn set_services(&mut self, list: Vec<Service>) {
        let unit = self.selected_service().map(|s| s.unit.clone());
        self.services = list;
        let at = unit.and_then(|u| self.visible_services().iter().position(|s| s.unit == u));
        self.follow(Tab::Services, at);
        self.clamp_all();
    }

    pub fn set_startup(&mut self, found: Startup) {
        self.startup = Some(found);
        self.clamp_all();
    }

    // Sorted, filtered processes and the app list, from the latest sample
    fn rebuild(&mut self) {
        let mut rows = views::filter(self.processes.clone(), &self.filter);
        views::sort(&mut rows, self.sort);
        self.tree_rows = if self.tree {
            views::tree(self.processes.clone(), self.sort, &self.filter)
        } else {
            Vec::new()
        };
        self.apps = views::apps(&rows, self.sort);
        self.processes_view = rows;
    }

    // Put a tab's cursor on a found row; a vanished row leaves it where it was
    fn follow(&mut self, tab: Tab, at: Option<usize>) {
        if let Some(i) = at {
            self.cursor[tab.index()] = i;
        }
    }

    // ── What is on screen ────────────────────────────────────────────────

    pub fn visible_processes(&self) -> &[Process] {
        &self.processes_view
    }

    pub fn visible_services(&self) -> Vec<&Service> {
        self.services
            .iter()
            .filter(|s| {
                self.filter.is_empty()
                    || s.unit.to_lowercase().contains(&self.filter.to_lowercase())
            })
            .collect()
    }

    // Rows in the current tab, for moving the cursor
    pub fn len(&self, tab: Tab) -> usize {
        match tab {
            Tab::Processes if self.tree => self.tree_rows.len(),
            Tab::Processes => self.processes_view.len(),
            Tab::Apps => self.apps.len(),
            Tab::Performance => 0,
            Tab::Services => self.visible_services().len(),
            Tab::Startup => self.startup.as_ref().map_or(0, |s| s.autostart.len()),
        }
    }

    pub fn cursor(&self) -> usize {
        self.cursor[self.tab.index()]
    }

    // Keep every cursor inside its list after the list shrank
    fn clamp_all(&mut self) {
        for tab in TABS {
            let len = self.len(tab);
            let c = &mut self.cursor[tab.index()];
            *c = (*c).min(len.saturating_sub(1));
        }
    }

    pub fn selected_process(&self) -> Option<&Process> {
        if self.tree {
            self.tree_rows
                .get(self.cursor[Tab::Processes.index()])
                .map(|r| &r.process)
        } else {
            self.processes_view.get(self.cursor[Tab::Processes.index()])
        }
    }

    fn selected_pid(&self) -> Option<u32> {
        self.selected_process().map(|p| p.pid)
    }

    pub fn selected_app(&self) -> Option<&App> {
        self.apps.get(self.cursor[Tab::Apps.index()])
    }

    pub fn selected_service(&self) -> Option<&Service> {
        self.visible_services()
            .get(self.cursor[Tab::Services.index()])
            .copied()
    }

    // ── Keys ─────────────────────────────────────────────────────────────

    pub fn key(&mut self, key: KeyEvent) -> Effect {
        // Ctrl+C always leaves, whatever is open
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Effect::Quit;
        }
        // a question is open: only y answers yes, anything else is no
        if let Some(action) = self.confirm.take() {
            return if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                Effect::Run(action)
            } else {
                Effect::None
            };
        }
        if self.editing_filter {
            return self.filter_key(key);
        }
        match key.code {
            KeyCode::Char('q') => return Effect::Quit,
            // Esc backs out one level: a filter first, then the program
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.refilter();
            }
            KeyCode::Esc => return Effect::Quit,
            KeyCode::Tab => return self.switch(TABS[(self.tab.index() + 1) % TABS.len()]),
            KeyCode::BackTab => {
                return self.switch(TABS[(self.tab.index() + TABS.len() - 1) % TABS.len()]);
            }
            KeyCode::Char(c @ '1'..='5') => return self.switch(TABS[c as usize - '1' as usize]),
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1),
            KeyCode::PageUp => self.move_by(-(PAGE as isize)),
            KeyCode::PageDown => self.move_by(PAGE as isize),
            KeyCode::Home | KeyCode::Char('g') => self.move_by(isize::MIN / 2),
            KeyCode::End | KeyCode::Char('G') => self.move_by(isize::MAX / 2),
            KeyCode::Char('/') if self.tab != Tab::Performance => self.editing_filter = true,
            _ => return self.tab_key(key.code),
        }
        Effect::None
    }

    // Typing into the filter line
    fn filter_key(&mut self, key: KeyEvent) -> Effect {
        match key.code {
            KeyCode::Enter => self.editing_filter = false,
            KeyCode::Esc => {
                self.editing_filter = false;
                self.filter.clear();
            }
            KeyCode::Backspace => {
                self.filter.pop();
            }
            KeyCode::Char(c) => self.filter.push(c),
            _ => {}
        }
        self.refilter();
        Effect::None
    }

    // The filter changed: redo the lists and start at the top
    fn refilter(&mut self) {
        self.rebuild();
        self.cursor[self.tab.index()] = 0;
        self.clamp_all();
    }

    fn switch(&mut self, tab: Tab) -> Effect {
        self.tab = tab;
        match tab {
            Tab::Services => Effect::LoadServices,
            Tab::Startup => Effect::LoadStartup,
            _ => Effect::None,
        }
    }

    fn move_by(&mut self, delta: isize) {
        let len = self.len(self.tab) as isize;
        let c = &mut self.cursor[self.tab.index()];
        *c = (*c as isize + delta).clamp(0, (len - 1).max(0)) as usize;
    }

    // Keys that only mean something on one tab
    fn tab_key(&mut self, code: KeyCode) -> Effect {
        match (self.tab, code) {
            (Tab::Processes | Tab::Apps, KeyCode::Char('s')) => {
                self.sort = next_sort(self.sort);
                self.rebuild();
            }
            (Tab::Processes, KeyCode::Char('t')) => {
                self.tree = !self.tree;
                self.rebuild();
                self.cursor[Tab::Processes.index()] = 0;
            }
            (Tab::Processes | Tab::Apps, KeyCode::Delete | KeyCode::Char('e')) => {
                self.ask_signal(Signal::Term)
            }
            (Tab::Processes | Tab::Apps, KeyCode::Char('K')) => self.ask_signal(Signal::Kill),
            (Tab::Processes | Tab::Apps, KeyCode::Char('p')) => self.ask_signal(Signal::Stop),
            (Tab::Processes | Tab::Apps, KeyCode::Char('c')) => {
                return self.now_signal(Signal::Cont);
            }
            // + makes a process gentler (nice up by one); the only direction allowed without root
            (Tab::Processes, KeyCode::Char('+') | KeyCode::Char('=')) => {
                if let Some(p) = self.selected_process() {
                    let action = Action::Renice {
                        pid: p.pid,
                        nice: (p.nice + 1).min(19),
                        label: format!("{} ({})", p.name, p.pid),
                    };
                    return Effect::Run(action);
                }
            }
            (Tab::Services, KeyCode::Char('u')) => {
                self.scope = if self.scope == Scope::User {
                    Scope::System
                } else {
                    Scope::User
                };
                self.cursor[Tab::Services.index()] = 0;
                return Effect::LoadServices;
            }
            (Tab::Services, KeyCode::Char('S')) => self.ask_service(ServiceVerb::Start),
            (Tab::Services, KeyCode::Char('X')) => self.ask_service(ServiceVerb::Stop),
            (Tab::Services, KeyCode::Char('R')) => self.ask_service(ServiceVerb::Restart),
            (Tab::Startup, KeyCode::Char(' ') | KeyCode::Enter) => {
                let entry = self
                    .startup
                    .as_ref()
                    .and_then(|s| s.autostart.get(self.cursor[Tab::Startup.index()]));
                if let Some(e) = entry {
                    return Effect::Run(Action::Startup {
                        id: e.id.clone(),
                        enable: !e.enabled,
                    });
                }
            }
            _ => {}
        }
        Effect::None
    }

    // The selected process, or every process of the selected app, as a signal action
    fn signal_target(&self, signal: Signal) -> Option<Action> {
        match self.tab {
            Tab::Processes => self.selected_process().map(|p| Action::Signal {
                pids: vec![p.pid],
                signal,
                label: format!("{} ({})", p.name, p.pid),
            }),
            Tab::Apps => self.selected_app().map(|a| Action::Signal {
                pids: a.pids.clone(),
                signal,
                label: format!(
                    "{} ({} process{})",
                    a.name,
                    a.processes,
                    if a.processes == 1 { "" } else { "es" }
                ),
            }),
            _ => None,
        }
    }

    fn ask_signal(&mut self, signal: Signal) {
        self.confirm = self.signal_target(signal);
    }

    fn now_signal(&mut self, signal: Signal) -> Effect {
        self.signal_target(signal).map_or(Effect::None, Effect::Run)
    }

    fn ask_service(&mut self, verb: ServiceVerb) {
        self.confirm = self.selected_service().map(|s| Action::Service {
            scope: s.scope,
            verb,
            unit: s.unit.clone(),
        });
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::system::Memory;
    use ratatui::crossterm::event::{KeyEventKind, KeyEventState};

    pub fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    pub fn proc_row(pid: u32, ppid: u32, name: &str, cpu: f64, app: &str) -> Process {
        Process {
            pid,
            ppid,
            name: name.into(),
            command: name.into(),
            user: "mgeist".into(),
            uid: Some(1000),
            state: "sleeping".into(),
            threads: 1,
            nice: 0,
            cpu_share: cpu,
            cpu_core: cpu,
            memory: 1024,
            read_rate: None,
            write_rate: None,
            gpu: None,
            app: app.into(),
            mine: true,
        }
    }

    pub fn totals() -> System {
        System {
            cpu: 12.0,
            cores: vec![10.0, 14.0],
            memory: Memory {
                total: 100,
                used: 40,
                ..Memory::default()
            },
            load: [0.5, 0.4, 0.3],
            uptime_seconds: 3600.0,
            rx_rate: 10.0,
            tx_rate: 5.0,
            interfaces: vec![],
            disks: vec![],
        }
    }

    pub fn loaded() -> State {
        let mut s = State::default();
        s.set_sample(
            vec![
                proc_row(1, 0, "systemd", 0.1, "init"),
                proc_row(20, 1, "firefox", 30.0, "firefox"),
                proc_row(21, 20, "Web Content", 5.0, "firefox"),
                proc_row(30, 1, "zsh", 1.0, "ghostty"),
            ],
            totals(),
        );
        s
    }

    #[test]
    fn the_cursor_stays_on_the_same_process_when_the_list_reorders() {
        let mut s = loaded();
        s.key(key(KeyCode::Down)); // firefox (30) → Web Content (5)
        assert_eq!(s.selected_process().unwrap().pid, 21);
        let mut rows = s.processes.clone();
        rows.iter_mut().find(|p| p.pid == 21).unwrap().cpu_share = 99.0;
        s.set_sample(rows, totals());
        assert_eq!(s.cursor(), 0, "Web Content moved to the top");
        assert_eq!(s.selected_process().unwrap().pid, 21);
    }

    #[test]
    fn ending_asks_first_and_only_y_says_yes() {
        let mut s = loaded();
        assert_eq!(s.key(key(KeyCode::Char('e'))), Effect::None);
        assert!(
            s.confirm
                .as_ref()
                .unwrap()
                .question()
                .starts_with("End firefox (20)")
        );
        assert_eq!(s.key(key(KeyCode::Char('n'))), Effect::None);
        assert!(s.confirm.is_none());
        s.key(key(KeyCode::Char('K')));
        let Effect::Run(Action::Signal { pids, signal, .. }) = s.key(key(KeyCode::Char('y')))
        else {
            panic!("y runs it")
        };
        assert_eq!((pids, signal), (vec![20], Signal::Kill));
    }

    #[test]
    fn pause_asks_first_continue_does_not_and_an_app_signals_all_its_processes() {
        let mut s = loaded();
        s.key(key(KeyCode::Char('2')));
        assert_eq!(
            s.key(key(KeyCode::Char('p'))),
            Effect::None,
            "pause can freeze the desktop — it asks"
        );
        assert!(
            s.confirm
                .as_ref()
                .unwrap()
                .question()
                .starts_with("Pause firefox (2 processes)")
        );
        let Effect::Run(Action::Signal { pids, signal, .. }) = s.key(key(KeyCode::Char('y')))
        else {
            panic!("y runs it")
        };
        assert_eq!((pids, signal), (vec![20, 21], Signal::Stop));
        let Effect::Run(Action::Signal { signal, .. }) = s.key(key(KeyCode::Char('c'))) else {
            panic!("continue needs no question")
        };
        assert_eq!(signal, Signal::Cont);
    }

    #[test]
    fn typing_a_filter_narrows_the_list_and_esc_backs_out_one_level() {
        let mut s = loaded();
        s.key(key(KeyCode::Char('/')));
        for c in "fire".chars() {
            s.key(key(KeyCode::Char(c)));
        }
        assert_eq!(
            s.key(key(KeyCode::Char('q'))),
            Effect::None,
            "q is a letter while typing"
        );
        s.key(key(KeyCode::Backspace));
        s.key(key(KeyCode::Enter));
        assert_eq!(s.filter, "fire");
        assert_eq!(
            s.visible_processes().len(),
            2,
            "firefox, and Web Content through its app"
        );
        assert_eq!(
            s.key(key(KeyCode::Esc)),
            Effect::None,
            "first Esc clears the filter"
        );
        assert_eq!(s.visible_processes().len(), 4);
        assert_eq!(s.key(key(KeyCode::Esc)), Effect::Quit);
    }

    #[test]
    fn sort_cycles_tree_nests_and_the_cursor_never_leaves_the_list() {
        let mut s = loaded();
        s.key(key(KeyCode::Char('s')));
        assert_eq!(s.sort, SortKey::Mem);
        s.key(key(KeyCode::Char('t')));
        assert_eq!(
            s.tree_rows.iter().map(|r| r.depth).collect::<Vec<_>>(),
            [0, 1, 2, 1]
        );
        s.key(key(KeyCode::End));
        assert_eq!(s.cursor(), 3);
        s.key(key(KeyCode::PageDown));
        assert_eq!(s.cursor(), 3);
        s.key(key(KeyCode::Home));
        assert_eq!(s.cursor(), 0);
    }

    #[test]
    fn tabs_ask_for_their_data_and_services_switch_scope() {
        let mut s = loaded();
        assert_eq!(s.key(key(KeyCode::Char('4'))), Effect::LoadServices);
        assert_eq!(s.key(key(KeyCode::Char('u'))), Effect::LoadServices);
        assert_eq!(s.scope, Scope::System);
        assert_eq!(s.key(key(KeyCode::Tab)), Effect::LoadStartup);
        assert_eq!(s.key(key(KeyCode::BackTab)), Effect::LoadServices);
        assert_eq!(s.key(key(KeyCode::Char('3'))), Effect::None);
        assert_eq!(s.tab, Tab::Performance);
    }

    #[test]
    fn plus_makes_a_process_gentler_never_faster() {
        let mut s = loaded();
        let Effect::Run(Action::Renice { pid, nice, .. }) = s.key(key(KeyCode::Char('+'))) else {
            panic!("renice")
        };
        assert_eq!((pid, nice), (20, 1));
    }

    #[test]
    fn ctrl_c_quits_even_mid_question() {
        let mut s = loaded();
        s.key(key(KeyCode::Char('e')));
        let ctrl_c = KeyEvent {
            modifiers: KeyModifiers::CONTROL,
            ..key(KeyCode::Char('c'))
        };
        assert_eq!(s.key(ctrl_c), Effect::Quit);
    }

    #[test]
    fn history_keeps_two_minutes() {
        let mut s = State::default();
        for _ in 0..HISTORY + 10 {
            s.set_sample(vec![], totals());
        }
        assert_eq!(s.history.cpu.len(), HISTORY);
        assert_eq!(s.history.memory.back(), Some(&40));
    }
}
