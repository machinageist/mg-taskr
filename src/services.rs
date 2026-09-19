// Author: Jeff
// Date: 2026-09-18
// Description: systemd services for the user or the whole machine, running or not
// Notes: Two systemctl questions merged: list-units knows what is loaded and running,
//        list-unit-files knows what is installed and whether it starts on its own.
//        A unit file nobody has loaded yet still belongs in the list, shown as stopped.
//        Template files (name@.service) are recipes, not services, so they are left out

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    User,
    System,
}

impl Scope {
    // The systemctl flag for this scope
    pub fn flag(self) -> &'static str {
        match self {
            Scope::User => "--user",
            Scope::System => "--system",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Service {
    pub unit: String,
    pub description: String,
    pub load: String,
    pub active: String,
    pub sub: String,
    // enabled, disabled, static, generated … or None when systemd has no file for it
    pub startup: Option<String>,
    pub scope: Scope,
}

#[derive(Deserialize)]
struct UnitRow {
    unit: String,
    load: String,
    active: String,
    sub: String,
    description: String,
}

#[derive(Deserialize)]
struct FileRow {
    unit_file: String,
    state: String,
}

// Merge the two systemctl answers into one list sorted by unit name
pub fn merge(units_json: &str, files_json: &str, scope: Scope) -> Result<Vec<Service>> {
    let units: Vec<UnitRow> =
        serde_json::from_str(units_json).context("reading systemctl list-units")?;
    let files: Vec<FileRow> =
        serde_json::from_str(files_json).context("reading systemctl list-unit-files")?;
    let startup: BTreeMap<String, String> = files
        .iter()
        .map(|f| (f.unit_file.clone(), f.state.clone()))
        .collect();

    let mut out: BTreeMap<String, Service> = BTreeMap::new();
    for u in units {
        // a running instance of a template takes its startup state from the template file
        let template = u
            .unit
            .split_once('@')
            .map(|(name, _)| format!("{name}@.service"));
        let state = startup
            .get(&u.unit)
            .or_else(|| template.and_then(|t| startup.get(&t)))
            .cloned();
        out.insert(
            u.unit.clone(),
            Service {
                unit: u.unit,
                description: u.description,
                load: u.load,
                active: u.active,
                sub: u.sub,
                startup: state,
                scope,
            },
        );
    }
    for f in files {
        if f.unit_file.ends_with("@.service") || out.contains_key(&f.unit_file) {
            continue;
        }
        out.insert(
            f.unit_file.clone(),
            Service {
                unit: f.unit_file,
                description: String::new(),
                load: "not-loaded".into(),
                active: "inactive".into(),
                sub: "dead".into(),
                startup: Some(f.state),
                scope,
            },
        );
    }
    Ok(out.into_values().collect())
}

// Run one systemctl query and return what it printed
fn systemctl(scope: Scope, args: &[&str]) -> Result<String> {
    let output = std::process::Command::new("systemctl")
        .arg(scope.flag())
        .args(args)
        .args(["--type=service", "--output=json", "--no-pager"])
        .output()
        .context("running systemctl")?;
    anyhow::ensure!(
        output.status.success(),
        "systemctl {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

// Every service in the scope, running or not
pub fn list(scope: Scope) -> Result<Vec<Service>> {
    let units = systemctl(scope, &["list-units", "--all"])?;
    let files = systemctl(scope, &["list-unit-files"])?;
    merge(&units, &files, scope)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNITS: &str = r#"[
        {"unit":"mpd.service","load":"loaded","active":"active","sub":"running","description":"Music Player Daemon"},
        {"unit":"app-blueman@autostart.service","load":"loaded","active":"inactive","sub":"dead","description":"Blueman"}
    ]"#;
    const FILES: &str = r#"[
        {"unit_file":"mpd.service","state":"enabled","preset":null},
        {"unit_file":"app-blueman@.service","state":"generated","preset":null},
        {"unit_file":"hypridle.service","state":"disabled","preset":"enabled"},
        {"unit_file":"getty@.service","state":"enabled","preset":null}
    ]"#;

    #[test]
    fn running_and_installed_services_merge_into_one_list() {
        let list = merge(UNITS, FILES, Scope::User).expect("parses");
        let names: Vec<&str> = list.iter().map(|s| s.unit.as_str()).collect();
        assert_eq!(
            names,
            [
                "app-blueman@autostart.service",
                "hypridle.service",
                "mpd.service"
            ],
            "templates are left out"
        );
        assert_eq!(list[2].startup.as_deref(), Some("enabled"));
        assert_eq!(
            list[1].active, "inactive",
            "installed but never loaded reads as stopped"
        );
        assert_eq!(
            list[0].startup.as_deref(),
            Some("generated"),
            "an instance takes its template's startup state"
        );
    }

    #[test]
    fn bad_json_is_an_error_not_an_empty_list() {
        assert!(merge("nope", "[]", Scope::System).is_err());
    }
}
