// Author: Jeff
// Date: 2026-09-18
// Description: `mg-taskr tui` — the terminal task manager's loop: sample, draw, read a key, act
// Notes: One look per second (the first after 300 ms so the screen fills quickly). Keys are
//        read between looks, so the screen answers at once. Services reload every 5 s while
//        their tab is open; startup reloads when its tab opens and after each toggle

mod draw;
pub mod state;

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

use crate::sampler::Sampler;
use crate::{actions, services, startup};
use state::{Action, Effect, State, Tab};

const TICK: Duration = Duration::from_secs(1);
const FIRST_TICK: Duration = Duration::from_millis(300);
const SERVICES_EVERY: Duration = Duration::from_secs(5);
const PROC_ROOT: &str = "/proc";
const SYS_ROOT: &str = "/sys";

// Take over the terminal, run until quit, and always give the terminal back
pub fn run() -> Result<()> {
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut DefaultTerminal) -> Result<()> {
    let mut sampler = Sampler::new(PROC_ROOT, SYS_ROOT);
    let mut state = State::default();
    let mut next_look = Instant::now() + FIRST_TICK;
    let mut services_loaded: Option<Instant> = None;
    loop {
        if Instant::now() >= next_look {
            let (rows, totals) = sampler.tick();
            state.set_sample(rows, totals);
            next_look = Instant::now() + TICK;
        }
        if state.tab == Tab::Services
            && services_loaded.is_none_or(|t| t.elapsed() >= SERVICES_EVERY)
        {
            load_services(&mut state);
            services_loaded = Some(Instant::now());
        }
        terminal.draw(|frame| draw::draw(frame, &state))?;

        if !event::poll(next_look.saturating_duration_since(Instant::now()))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        // terminals that report releases would otherwise act twice per press
        if key.kind != KeyEventKind::Press {
            continue;
        }
        // any key clears the last result line
        state.message = None;
        match state.key(key) {
            Effect::None => {}
            Effect::Quit => return Ok(()),
            Effect::LoadServices => {
                load_services(&mut state);
                services_loaded = Some(Instant::now());
            }
            Effect::LoadStartup => state.set_startup(startup::list()),
            Effect::Run(action) => {
                let reload_services = matches!(action, Action::Service { .. });
                let reload_startup = matches!(action, Action::Startup { .. });
                state.message = Some(match act(action) {
                    Ok(done) => (done, false),
                    Err(e) => (format!("{e:#}"), true),
                });
                if reload_services {
                    load_services(&mut state);
                }
                if reload_startup {
                    state.set_startup(startup::list());
                }
                // show the effect (a paused or ended process) without waiting a full second
                next_look = Instant::now();
            }
        }
    }
}

// Reload the services list for the chosen scope; a failure shows on the status line
fn load_services(state: &mut State) {
    match services::list(state.scope) {
        Ok(list) => state.set_services(list),
        Err(e) => state.message = Some((format!("{e:#}"), true)),
    }
}

// Carry out one action and say what happened
fn act(action: Action) -> Result<String> {
    let proc_root = Path::new(PROC_ROOT);
    match action {
        Action::Signal {
            pids,
            signal,
            label,
        } => actions::signal_many(proc_root, &pids, signal, &label),
        Action::Renice { pid, nice, .. } => actions::renice(proc_root, pid, nice),
        Action::Service { scope, verb, unit } => actions::service(scope, verb, &unit),
        Action::Startup { id, enable } => startup::set_enabled_here(&id, enable),
    }
}
