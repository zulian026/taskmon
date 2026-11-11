//! systemd services via `systemctl list-units --output=json`.
//! Listing needs no privileges. Start/stop/restart on system services
//! may prompt for authentication or fail; the error is surfaced as-is.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceView {
    pub unit: String,
    pub load: String,
    pub active: String,
    pub sub: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceActionResult {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

pub fn validate_scope(scope: &str) -> Result<bool, String> {
    match scope {
        "user" => Ok(true),
        "system" => Ok(false),
        _ => Err("scope must be 'user' or 'system'".to_string()),
    }
}

pub fn validate_unit(unit: &str) -> Result<(), String> {
    if !unit.ends_with(".service") {
        return Err("unit must end with .service".to_string());
    }
    if unit.len() > 256
        || !unit
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_@.:".contains(c))
    {
        return Err("invalid unit name".to_string());
    }
    Ok(())
}

pub fn list_services(scope: &str) -> Result<Vec<ServiceView>, String> {
    let user = validate_scope(scope)?;
    let mut cmd = std::process::Command::new("systemctl");
    if user {
        cmd.arg("--user");
    }
    let out = cmd
        .args(["list-units", "--type=service", "--all", "--output=json", "--no-pager"])
        .output()
        .map_err(|e| format!("cannot run systemctl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "systemctl failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    parse_services_json(&text)
}

pub fn parse_services_json(text: &str) -> Result<Vec<ServiceView>, String> {
    let mut services: Vec<ServiceView> =
        serde_json::from_str(text).map_err(|e| format!("cannot parse systemctl output: {e}"))?;
    services.sort_by(|a, b| a.unit.cmp(&b.unit));
    Ok(services)
}

pub fn service_action(scope: &str, unit: &str, action: &str) -> Result<ServiceActionResult, String> {
    let user = validate_scope(scope)?;
    validate_unit(unit)?;
    if !matches!(action, "start" | "stop" | "restart" | "enable" | "disable") {
        return Err("action must be start, stop, restart, enable or disable".to_string());
    }
    let mut cmd = std::process::Command::new("systemctl");
    if user {
        cmd.arg("--user");
    }
    let out = cmd
        .arg(action)
        .arg(unit)
        .output()
        .map_err(|e| format!("cannot run systemctl: {e}"))?;
    Ok(ServiceActionResult {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).trim().to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_systemctl_json() {
        let text = r#"[{"unit":"cron.service","load":"loaded","active":"active","sub":"running","description":"Regular background program processing daemon"},{"unit":"foo.service","load":"not-found","active":"inactive","sub":"dead","description":""}]"#;
        let list = parse_services_json(text).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].unit, "cron.service");
        assert_eq!(list[0].sub, "running");
    }

    #[test]
    fn rejects_bad_units() {
        assert!(validate_unit("foo.service").is_ok());
        assert!(validate_unit("foo; rm -rf /").is_err());
        assert!(validate_unit("foo.timer").is_err());
        assert!(validate_unit("../etc/passwd").is_err());
    }

    #[test]
    fn rejects_bad_scope_and_action() {
        assert!(validate_scope("user").is_ok());
        assert!(validate_scope("root").is_err());
        assert!(service_action("user", "x.service", "explode").is_err());
    }
}
