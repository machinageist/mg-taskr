// Author: Jeff
// Date: 2026-09-18
// Description: mg-taskr core — reads what the machine is running, for the CLI, TUI and shell panel
// Notes: procfs parses the raw files, sample turns two looks into per-process rates, system does
//        the same for machine-wide totals, sampler keeps the last look for live views, views
//        sorts/filters/groups, actions changes things, tui draws it all, os holds the only
//        unsafe calls

pub mod actions;
pub mod os;
pub mod procfs;
pub mod sample;
pub mod sampler;
pub mod services;
pub mod startup;
pub mod system;
pub mod tui;
pub mod units;
pub mod views;
