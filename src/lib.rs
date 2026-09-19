// Author: Jeff
// Date: 2026-09-18
// Description: mg-taskr core — reads what the machine is running, for the CLI, TUI and shell panel
// Notes: procfs parses the raw files, sample turns two looks into per-process rates, system does
//        the same for machine-wide totals, os holds the only unsafe calls

pub mod os;
pub mod procfs;
pub mod sample;
pub mod system;
