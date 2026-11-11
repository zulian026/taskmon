//! Startup apps: freedesktop .desktop autostart entries.
//! User entries live in ~/.config/autostart, system entries in
//! /etc/xdg/autostart. Disabling a system entry is done the standard way:
//! a user override copy with Hidden=true. Enabling removes that override.

use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct StartupEntry {
    pub id: String,
    pub name: String,
    pub exec: String,
    pub comment: Option<String>,
    pub source: String, // "user" | "system"
    pub enabled: bool,
    pub path: String,
}

fn user_dir() -> PathBuf {
    dirs_home().join(".config").join("autostart")
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/root"))
}

fn system_dir() -> PathBuf {
    PathBuf::from("/etc/xdg/autostart")
}

/// Parse the [Desktop Entry] section of a .desktop file into key/value pairs.
pub fn parse_desktop(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    map
}

fn entry_from_file(path: &Path, source: &str) -> Option<StartupEntry> {
    let text = std::fs::read_to_string(path).ok()?;
    let kv = parse_desktop(&text);
    let exec = kv.get("Exec")?;
    if exec.is_empty() {
        return None;
    }
    // NoDisplay entries are hidden from startup managers; skip them.
    if kv.get("NoDisplay").map(|v| v == "true").unwrap_or(false) {
        return None;
    }
    let hidden = kv.get("Hidden").map(|v| v == "true").unwrap_or(false);
    let gnome_off = kv
        .get("X-GNOME-Autostart-enabled")
        .map(|v| v == "false")
        .unwrap_or(false);
    Some(StartupEntry {
        id: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        name: kv
            .get("Name")
            .cloned()
            .unwrap_or_else(|| exec.clone()),
        exec: exec.clone(),
        comment: kv.get("Comment").cloned(),
        source: source.to_string(),
        enabled: !hidden && !gnome_off,
        path: path.to_string_lossy().into_owned(),
    })
}

pub fn list_startup() -> Vec<StartupEntry> {
    let mut by_id: HashMap<String, StartupEntry> = HashMap::new();
    // System first, then user overrides by the same filename.
    for dir in [system_dir(), user_dir()] {
        let source = if dir == system_dir() { "system" } else { "user" };
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for ent in rd.flatten() {
            let path = ent.path();
            if path.extension().map(|e| e != "desktop").unwrap_or(true) {
                continue;
            }
            if let Some(e) = entry_from_file(&path, source) {
                by_id.insert(e.id.clone(), e);
            }
        }
    }
    let mut out: Vec<StartupEntry> = by_id.into_values().collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// Enable/disable a startup entry by its .desktop filename.
/// Returns the updated entry.
pub fn set_startup(id: &str, enabled: bool) -> Result<StartupEntry, String> {
    if id.contains('/') || id.contains('\\') || !id.ends_with(".desktop") {
        return Err("invalid startup entry id".to_string());
    }
    let user_path = user_dir().join(id);
    let system_path = system_dir().join(id);
    if user_path.exists() {
        set_hidden(&user_path, !enabled)?;
    } else if system_path.exists() {
        if !enabled {
            std::fs::create_dir_all(user_dir())
                .map_err(|e| format!("cannot create autostart dir: {e}"))?;
            let text = std::fs::read_to_string(&system_path)
                .map_err(|e| format!("cannot read {id}: {e}"))?;
            let patched = set_hidden_in_text(&text, true);
            std::fs::write(&user_path, patched)
                .map_err(|e| format!("cannot write override: {e}"))?;
        }
        // Enabling a system-only entry needs no override.
    } else {
        return Err(format!("startup entry {id} not found"));
    }
    list_startup()
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| format!("startup entry {id} disappeared after update"))
}

/// Set Hidden=<value> in a .desktop file on disk.
fn set_hidden(path: &Path, hidden: bool) -> Result<(), String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let patched = set_hidden_in_text(&text, hidden);
    std::fs::write(path, patched).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Set or remove the Hidden= line in .desktop text.
/// Disabling adds Hidden=true; enabling removes the line entirely.
pub fn set_hidden_in_text(text: &str, hidden: bool) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut saw_hidden = false;
    let mut in_entry = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_entry = trimmed == "[Desktop Entry]";
            out.push(line.to_string());
            continue;
        }
        if in_entry && trimmed.starts_with("Hidden=") {
            saw_hidden = true;
            if hidden {
                out.push("Hidden=true".to_string());
            }
            continue;
        }
        out.push(line.to_string());
    }
    if hidden && !saw_hidden {
        // Insert right after the [Desktop Entry] header.
        let mut placed = false;
        let mut final_out: Vec<String> = Vec::new();
        for line in out {
            final_out.push(line.clone());
            if !placed && line.trim() == "[Desktop Entry]" {
                final_out.push("Hidden=true".to_string());
                placed = true;
            }
        }
        if !placed {
            final_out.insert(0, "[Desktop Entry]".to_string());
            final_out.insert(1, "Hidden=true".to_string());
        }
        return final_out.join("\n") + "\n";
    }
    out.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_desktop_reads_entry_section_only() {
        let text = "[Desktop Entry]\nName=Foo\nExec=/usr/bin/foo --x\nHidden=false\n\n[Other]\nName=Bar\n";
        let kv = parse_desktop(text);
        assert_eq!(kv.get("Name").map(|s| s.as_str()), Some("Foo"));
        assert_eq!(kv.get("Exec").map(|s| s.as_str()), Some("/usr/bin/foo --x"));
    }

    #[test]
    fn hidden_toggle_roundtrip() {
        let text = "[Desktop Entry]\nName=Foo\nExec=/usr/bin/foo\n";
        let off = set_hidden_in_text(text, true);
        assert!(off.contains("Hidden=true"));
        let kv = parse_desktop(&off);
        assert_eq!(kv.get("Hidden").map(|s| s.as_str()), Some("true"));
        let on = set_hidden_in_text(&off, false);
        assert!(!on.contains("Hidden="));
        assert!(on.contains("Name=Foo"));
    }

    #[test]
    fn hidden_toggle_replaces_existing_line() {
        let text = "[Desktop Entry]\nName=Foo\nHidden=false\nExec=/usr/bin/foo\n";
        let off = set_hidden_in_text(text, true);
        assert_eq!(off.matches("Hidden=").count(), 1);
        assert!(off.contains("Hidden=true"));
    }
}
