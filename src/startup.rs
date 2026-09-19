// Author: Jeff
// Date: 2026-09-18
// Description: What starts at login — XDG autostart entries, plus Hyprland's own autostart lines
// Notes: XDG rule: a file in ~/.config/autostart replaces the system file with the same name,
//        and Hidden=true (or X-GNOME-Autostart-enabled=false) turns an entry off. That is how
//        slice 3 will disable a system entry without touching /etc.
//        OnlyShowIn/NotShowIn decide whether an entry runs under this desktop at all.
//        Hyprland's lines live in its Lua config; they are shown, never edited here

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub exec: String,
    pub comment: String,
    // "user" (~/.config/autostart) or "system" (/etc/xdg/autostart)
    pub source: String,
    // a user file replaces a system one of the same name
    pub overrides_system: bool,
    pub enabled: bool,
    // false when OnlyShowIn/NotShowIn rule out the current desktop
    pub runs_here: bool,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HyprLine {
    pub file: PathBuf,
    pub line: usize,
    pub command: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Startup {
    pub autostart: Vec<Entry>,
    pub hyprland: Vec<HyprLine>,
}

// ── Desktop files ────────────────────────────────────────────────────────

// The key=value pairs of the [Desktop Entry] group; other groups (actions) are ignored
pub fn parse_desktop(text: &str) -> BTreeMap<String, String> {
    let mut in_entry = false;
    let mut keys = BTreeMap::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if in_entry
            && !line.starts_with('#')
            && let Some((key, value)) = line.split_once('=')
        {
            keys.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    keys
}

// Does a ;-separated desktop list include any of ours
fn lists_any(list: Option<&String>, ours: &[&str]) -> bool {
    list.is_some_and(|l| {
        l.split(';')
            .any(|d| !d.is_empty() && ours.iter().any(|o| o.eq_ignore_ascii_case(d)))
    })
}

// One autostart file → an entry; `desktops` is XDG_CURRENT_DESKTOP split on ':'
pub fn entry(
    id: &str,
    keys: &BTreeMap<String, String>,
    source: &str,
    path: &Path,
    desktops: &[&str],
) -> Entry {
    let get = |k: &str| keys.get(k).cloned().unwrap_or_default();
    let hidden = get("Hidden").eq_ignore_ascii_case("true");
    let gnome_off = get("X-GNOME-Autostart-enabled").eq_ignore_ascii_case("false");
    let only = keys.get("OnlyShowIn");
    let runs_here = (only.is_none() || lists_any(only, desktops))
        && !lists_any(keys.get("NotShowIn"), desktops);
    Entry {
        id: id.to_string(),
        name: keys.get("Name").cloned().unwrap_or_else(|| id.to_string()),
        exec: get("Exec"),
        comment: get("Comment"),
        source: source.to_string(),
        overrides_system: false,
        enabled: !hidden && !gnome_off,
        runs_here,
        path: path.to_path_buf(),
    }
}

// Every *.desktop file in one folder as (id, path), sorted
fn desktop_files(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(String, PathBuf)> = read
        .filter_map(|e| {
            let path = e.ok()?.path();
            let id = path
                .file_name()?
                .to_str()?
                .strip_suffix(".desktop")?
                .to_string();
            Some((id, path))
        })
        .collect();
    files.sort();
    files
}

// All autostart entries: system folders first, then the user folder replaces by id
// earlier system folders win over later ones, as XDG_CONFIG_DIRS orders them
pub fn autostart(user_dir: &Path, system_dirs: &[PathBuf], desktops: &[&str]) -> Vec<Entry> {
    let mut by_id: BTreeMap<String, Entry> = BTreeMap::new();
    for dir in system_dirs.iter().rev() {
        for (id, path) in desktop_files(dir) {
            let keys = parse_desktop(&std::fs::read_to_string(&path).unwrap_or_default());
            by_id.insert(id.clone(), entry(&id, &keys, "system", &path, desktops));
        }
    }
    for (id, path) in desktop_files(user_dir) {
        let keys = parse_desktop(&std::fs::read_to_string(&path).unwrap_or_default());
        let mut e = entry(&id, &keys, "user", &path, desktops);
        e.overrides_system = by_id.contains_key(&id);
        by_id.insert(id, e);
    }
    by_id.into_values().collect()
}

// ── Turning entries on and off ───────────────────────────────────────────

// Set one key inside [Desktop Entry], replacing it if present, else adding it at the group's end
// other groups ([Desktop Action …]) are left exactly as they were
pub fn set_key(text: &str, key: &str, value: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_entry = false;
    let mut done = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            // leaving the entry group without having seen the key — add it before the next group
            if in_entry && !done {
                out.push(format!("{key}={value}"));
                done = true;
            }
            in_entry = trimmed == "[Desktop Entry]";
        } else if in_entry
            && !done
            && trimmed
                .split_once('=')
                .is_some_and(|(k, _)| k.trim() == key)
        {
            out.push(format!("{key}={value}"));
            done = true;
            continue;
        }
        out.push(line.to_string());
    }
    if !done {
        if !in_entry {
            out.push("[Desktop Entry]".into());
        }
        out.push(format!("{key}={value}"));
    }
    out.join("\n") + "\n"
}

// An id is a file name, never a path
fn valid_id(id: &str) -> bool {
    !id.is_empty() && !id.starts_with('.') && !id.contains('/') && !id.contains('\0')
}

// Turn an autostart entry on or off by writing the user's copy
// a system entry is copied into the user folder first; /etc is never touched
pub fn set_enabled(
    id: &str,
    enabled: bool,
    user_dir: &Path,
    system_dirs: &[PathBuf],
) -> Result<String> {
    if !valid_id(id) {
        bail!("{id:?} is not an autostart id");
    }
    let user_path = user_dir.join(format!("{id}.desktop"));
    let source = if user_path.exists() {
        user_path.clone()
    } else {
        system_dirs
            .iter()
            .map(|d| d.join(format!("{id}.desktop")))
            .find(|p| p.exists())
            .with_context(|| format!("no autostart entry named {id}"))?
    };
    let text = std::fs::read_to_string(&source)
        .with_context(|| format!("reading {}", source.display()))?;
    let mut next = set_key(&text, "Hidden", if enabled { "false" } else { "true" });
    // GNOME's own off switch would keep it off after we cleared Hidden
    if enabled
        && parse_desktop(&next)
            .get("X-GNOME-Autostart-enabled")
            .is_some_and(|v| v.eq_ignore_ascii_case("false"))
    {
        next = set_key(&next, "X-GNOME-Autostart-enabled", "true");
    }
    std::fs::create_dir_all(user_dir)
        .with_context(|| format!("creating {}", user_dir.display()))?;
    // write beside it, then rename — a crash never leaves half a file
    let tmp = user_dir.join(format!(".{id}.desktop.tmp"));
    std::fs::write(&tmp, next).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &user_path).with_context(|| format!("saving {}", user_path.display()))?;
    Ok(format!(
        "{id} {}",
        if enabled { "enabled" } else { "disabled" }
    ))
}

// Enable or disable in the real folders
pub fn set_enabled_here(id: &str, enabled: bool) -> Result<String> {
    set_enabled(
        id,
        enabled,
        &config_home().join("autostart"),
        &system_autostart_dirs(),
    )
}

// ── Hyprland ─────────────────────────────────────────────────────────────

// The quoted command inside hl.exec_cmd("…") on one line, if there is one
fn exec_cmd_arg(line: &str) -> Option<String> {
    let rest = line.split_once("hl.exec_cmd(")?.1.trim_start();
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let mut out = String::new();
    let mut chars = rest[1..].chars();
    // walk to the matching quote, keeping escaped characters as written
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push(chars.next()?),
            c if c == quote => return Some(out),
            c => out.push(c),
        }
    }
    None
}

// Commands Hyprland starts from its Lua autostart file, with line numbers; comments skipped
pub fn hyprland_lines(file: &Path, text: &str) -> Vec<HyprLine> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim_start().starts_with("--"))
        .filter_map(|(i, line)| {
            Some(HyprLine {
                file: file.to_path_buf(),
                line: i + 1,
                command: exec_cmd_arg(line)?,
            })
        })
        .collect()
}

// ── Where things live on this machine ────────────────────────────────────

// $XDG_CONFIG_HOME, or ~/.config
pub fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        })
}

// $XDG_CONFIG_DIRS/autostart for each entry, or /etc/xdg/autostart
fn system_autostart_dirs() -> Vec<PathBuf> {
    let dirs = std::env::var("XDG_CONFIG_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/etc/xdg".into());
    dirs.split(':')
        .filter(|d| !d.is_empty())
        .map(|d| Path::new(d).join("autostart"))
        .collect()
}

// Everything that starts at login, read from the real folders
pub fn list() -> Startup {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let desktops: Vec<&str> = desktop.split(':').filter(|d| !d.is_empty()).collect();
    let hypr = config_home().join("hypr/autostart.lua");
    Startup {
        autostart: autostart(
            &config_home().join("autostart"),
            &system_autostart_dirs(),
            &desktops,
        ),
        hyprland: std::fs::read_to_string(&hypr)
            .map(|t| hyprland_lines(&hypr, &t))
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_files_read_only_the_main_group() {
        let keys = parse_desktop(
            "# c\n[Desktop Entry]\nName=Blueman\nExec=blueman-applet\n[Desktop Action x]\nName=Other\n",
        );
        assert_eq!(keys.get("Name").map(String::as_str), Some("Blueman"));
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn hidden_or_gnome_off_means_disabled_and_show_rules_follow_the_desktop() {
        let p = Path::new("/x.desktop");
        let e = |text: &str| entry("x", &parse_desktop(text), "system", p, &["Hyprland"]);
        assert!(e("[Desktop Entry]\nName=a\n").enabled);
        assert!(!e("[Desktop Entry]\nHidden=true\n").enabled);
        assert!(!e("[Desktop Entry]\nX-GNOME-Autostart-enabled=false\n").enabled);
        assert!(!e("[Desktop Entry]\nOnlyShowIn=GNOME;KDE;\n").runs_here);
        assert!(e("[Desktop Entry]\nOnlyShowIn=GNOME;hyprland;\n").runs_here);
        assert!(!e("[Desktop Entry]\nNotShowIn=Hyprland;\n").runs_here);
        assert_eq!(
            e("[Desktop Entry]\n").name,
            "x",
            "no Name falls back to the id"
        );
    }

    #[test]
    fn a_user_file_replaces_the_system_one() {
        let root = std::env::temp_dir().join(format!("mg-taskr-startup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (user, system) = (root.join("user"), root.join("system"));
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&system).unwrap();
        std::fs::write(
            system.join("nm-applet.desktop"),
            "[Desktop Entry]\nName=NM\nExec=nm-applet\n",
        )
        .unwrap();
        std::fs::write(
            system.join("blueman.desktop"),
            "[Desktop Entry]\nName=Blueman\n",
        )
        .unwrap();
        std::fs::write(
            user.join("nm-applet.desktop"),
            "[Desktop Entry]\nName=NM\nHidden=true\n",
        )
        .unwrap();
        let list = autostart(&user, &[system], &["Hyprland"]);
        assert_eq!(list.len(), 2);
        let nm = list.iter().find(|e| e.id == "nm-applet").unwrap();
        assert!(!nm.enabled && nm.overrides_system && nm.source == "user");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn set_key_replaces_or_adds_inside_the_entry_group_only() {
        let text = "[Desktop Entry]\nName=NM\nHidden=false\n[Desktop Action x]\nHidden=keep\n";
        assert_eq!(
            set_key(text, "Hidden", "true"),
            "[Desktop Entry]\nName=NM\nHidden=true\n[Desktop Action x]\nHidden=keep\n"
        );
        assert_eq!(
            set_key(
                "[Desktop Entry]\nName=NM\n[Desktop Action x]\n",
                "Hidden",
                "true"
            ),
            "[Desktop Entry]\nName=NM\nHidden=true\n[Desktop Action x]\n"
        );
        assert_eq!(
            set_key("[Desktop Entry]\nName=NM", "Hidden", "true"),
            "[Desktop Entry]\nName=NM\nHidden=true\n"
        );
        assert_eq!(
            set_key("", "Hidden", "true"),
            "[Desktop Entry]\nHidden=true\n"
        );
    }

    #[test]
    fn disabling_a_system_entry_writes_a_user_copy_and_enabling_clears_both_switches() {
        let root = std::env::temp_dir().join(format!("mg-taskr-toggle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (user, system) = (root.join("user"), root.join("system"));
        std::fs::create_dir_all(&system).unwrap();
        let original =
            "[Desktop Entry]\nName=NM\nExec=nm-applet\nX-GNOME-Autostart-enabled=false\n";
        std::fs::write(system.join("nm-applet.desktop"), original).unwrap();
        let dirs = [system.clone()];

        set_enabled("nm-applet", false, &user, &dirs).unwrap();
        assert_eq!(
            std::fs::read_to_string(system.join("nm-applet.desktop")).unwrap(),
            original,
            "system file untouched"
        );
        let list = autostart(&user, &dirs, &[]);
        assert!(!list[0].enabled && list[0].overrides_system);

        set_enabled("nm-applet", true, &user, &dirs).unwrap();
        assert!(autostart(&user, &dirs, &[])[0].enabled);
        assert!(set_enabled("../evil", false, &user, &dirs).is_err());
        assert!(set_enabled("missing", false, &user, &dirs).is_err());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn hyprland_exec_lines_are_found_and_comments_skipped() {
        let lua = "hl.on(\"hyprland.start\", function ()\n    hl.exec_cmd(\"hyprpaper\")\n    -- hl.exec_cmd(\"off\")\n    hl.exec_cmd('a \\'quoted\\' b')\nend)\n";
        let lines = hyprland_lines(Path::new("a.lua"), lua);
        let found: Vec<(usize, &str)> =
            lines.iter().map(|l| (l.line, l.command.as_str())).collect();
        assert_eq!(found, [(2, "hyprpaper"), (4, "a 'quoted' b")]);
    }
}
