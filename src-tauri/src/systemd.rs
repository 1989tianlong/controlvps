use std::collections::HashMap;

use ssh2::Session;

use crate::error::{AppError, AppResult};
use crate::models::{
    SystemdAction, SystemdActionResult, SystemdLogs, SystemdOverview, SystemdService,
};
use crate::ssh::{self, ExecOutput};

pub fn list(session: &Session) -> AppResult<SystemdOverview> {
    let version = execute_checked(session, "systemctl --version")?;
    let listing = execute_checked(
        session,
        "systemctl list-units --type=service --all --no-pager --no-legend --plain",
    )?;
    let mut services = listing
        .stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(parse_service)
        .collect::<AppResult<Vec<_>>>()?;
    let unit_files = execute_checked(
        session,
        "systemctl list-unit-files --type=service --no-pager --no-legend --plain",
    )?;
    let mut enabled_states = unit_files
        .stdout
        .lines()
        .filter_map(parse_unit_file)
        .collect::<HashMap<_, _>>();
    for service in &mut services {
        if let Some(enabled) = enabled_states.remove(&service.name) {
            service.enabled = enabled;
        }
    }
    services.extend(
        enabled_states
            .into_iter()
            .map(|(name, enabled)| SystemdService {
                name,
                load: "loaded".into(),
                active: "inactive".into(),
                sub: "dead".into(),
                description: String::new(),
                enabled,
            }),
    );
    services.sort_by(|left, right| {
        right
            .active
            .eq("active")
            .cmp(&left.active.eq("active"))
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(SystemdOverview {
        version: version
            .stdout
            .lines()
            .next()
            .unwrap_or("systemd")
            .trim()
            .to_string(),
        services,
    })
}

pub fn action(
    session: &Session,
    unit: &str,
    action: SystemdAction,
) -> AppResult<SystemdActionResult> {
    validate_unit(unit)?;
    let action_name = match action {
        SystemdAction::Start => "start",
        SystemdAction::Stop => "stop",
        SystemdAction::Restart => "restart",
    };
    let output = execute_checked(session, &format!("systemctl {action_name} -- {unit}"))?;
    Ok(SystemdActionResult {
        unit: unit.to_string(),
        action,
        output: combined_output(&output),
        exit_code: output.exit_code,
    })
}

pub fn logs(session: &Session, unit: &str) -> AppResult<SystemdLogs> {
    validate_unit(unit)?;
    let output = execute_checked(
        session,
        &format!("journalctl -u {unit} -n 200 --no-pager --output=short-iso"),
    )?;
    Ok(SystemdLogs {
        unit: unit.to_string(),
        output: combined_output(&output),
    })
}

fn execute_checked(session: &Session, command: &str) -> AppResult<ExecOutput> {
    let output = ssh::execute(session, command)?;
    if output.exit_code != 0 {
        let details = combined_output(&output);
        return Err(AppError::Message(format!(
            "无法访问 systemd，请确认服务器使用 systemd 且当前用户有足够权限：{}",
            details.trim()
        )));
    }
    Ok(output)
}

fn parse_service(line: &str) -> AppResult<SystemdService> {
    let line = line.trim();
    let line = line.strip_prefix('●').unwrap_or(line).trim_start();
    let mut columns = line.split_whitespace();
    let mut required = |name: &str| {
        columns
            .next()
            .map(str::to_string)
            .ok_or_else(|| AppError::Message(format!("systemd 服务列表缺少字段 {name}")))
    };
    let service = SystemdService {
        name: required("UNIT")?,
        load: required("LOAD")?,
        active: required("ACTIVE")?,
        sub: required("SUB")?,
        description: columns.collect::<Vec<_>>().join(" "),
        enabled: "unknown".into(),
    };
    validate_unit(&service.name)?;
    Ok(service)
}

fn parse_unit_file(line: &str) -> Option<(String, String)> {
    let mut columns = line.split_whitespace();
    let name = columns.next()?.to_string();
    let enabled = columns.next()?.to_string();
    validate_unit(&name).ok()?;
    Some((name, enabled))
}

fn validate_unit(unit: &str) -> AppResult<()> {
    let valid = !unit.is_empty()
        && unit.len() <= 256
        && unit.ends_with(".service")
        && unit
            .chars()
            .next()
            .is_some_and(|value| value.is_ascii_alphanumeric())
        && unit.chars().all(|value| {
            value.is_ascii_alphanumeric() || matches!(value, '-' | '_' | '.' | '@' | ':' | '\\')
        });
    if !valid {
        return Err(AppError::Message("systemd 服务名称无效".into()));
    }
    Ok(())
}

fn combined_output(output: &ExecOutput) -> String {
    let mut text = output.stdout.clone();
    if !output.stderr.trim().is_empty() {
        text.push_str(&output.stderr);
    }
    text
}
