use std::collections::HashMap;

use serde_json::Value;
use ssh2::Session;

use crate::error::{AppError, AppResult};
use crate::models::{
    DockerAction, DockerActionResult, DockerContainer, DockerLogs, DockerOverview,
};
use crate::ssh::{self, ExecOutput};

pub fn list(session: &Session) -> AppResult<DockerOverview> {
    let version = execute_checked(session, "docker version --format '{{.Server.Version}}'")?;
    let listing = execute_checked(session, "docker ps -a --no-trunc --format '{{json .}}'")?;
    let mut containers = listing
        .stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(parse_container)
        .collect::<AppResult<Vec<_>>>()?;
    let stats = ssh::execute(
        session,
        "docker stats --no-stream --no-trunc --format '{{json .}}'",
    );
    if let Ok(stats) = stats {
        let stats_by_id = stats
            .stdout
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter_map(|value| {
                let id = value.get("ID").and_then(Value::as_str)?.to_string();
                Some((id, value))
            })
            .collect::<HashMap<_, _>>();
        for container in &mut containers {
            if let Some(stats) = stats_by_id.get(&container.id) {
                container.cpu_percent = percentage(stats, "CPUPerc");
                container.memory_percent = percentage(stats, "MemPerc");
                container.memory_usage = stats
                    .get("MemUsage")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                container.pids = stats
                    .get("PIDs")
                    .and_then(Value::as_str)
                    .and_then(|value| value.parse().ok());
            }
        }
    }
    containers.sort_by(|left, right| {
        right
            .state
            .eq("running")
            .cmp(&left.state.eq("running"))
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(DockerOverview {
        version: version.stdout.trim().to_string(),
        containers,
    })
}

pub fn action(
    session: &Session,
    container_id: &str,
    action: DockerAction,
) -> AppResult<DockerActionResult> {
    validate_container_id(container_id)?;
    let action_name = match action {
        DockerAction::Start => "start",
        DockerAction::Stop => "stop",
        DockerAction::Restart => "restart",
    };
    let output = execute_checked(session, &format!("docker {action_name} {container_id}"))?;
    Ok(DockerActionResult {
        container_id: container_id.to_string(),
        action,
        output: combined_output(&output),
        exit_code: output.exit_code,
    })
}

pub fn logs(session: &Session, container_id: &str) -> AppResult<DockerLogs> {
    validate_container_id(container_id)?;
    let output = execute_checked(session, &format!("docker logs --tail 200 {container_id}"))?;
    Ok(DockerLogs {
        container_id: container_id.to_string(),
        output: combined_output(&output),
    })
}

fn execute_checked(session: &Session, command: &str) -> AppResult<ExecOutput> {
    let output = ssh::execute(session, command)?;
    if output.exit_code != 0 {
        let details = combined_output(&output);
        return Err(AppError::Message(format!(
            "无法访问 Docker，请确认已安装 Docker 且当前用户有权限：{}",
            details.trim()
        )));
    }
    Ok(output)
}

fn parse_container(line: &str) -> AppResult<DockerContainer> {
    let value = serde_json::from_str::<Value>(line)
        .map_err(|error| AppError::Message(format!("无法解析 Docker 容器列表：{error}")))?;
    let required = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| AppError::Message(format!("Docker 容器列表缺少字段 {key}")))
    };
    Ok(DockerContainer {
        id: required("ID")?,
        name: required("Names")?,
        image: required("Image")?,
        state: required("State")?,
        status: required("Status")?,
        ports: value
            .get("Ports")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        created_at: value
            .get("CreatedAt")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        cpu_percent: None,
        memory_percent: None,
        memory_usage: None,
        pids: None,
    })
}

fn percentage(value: &Value, key: &str) -> Option<f64> {
    value
        .get(key)
        .and_then(Value::as_str)
        .and_then(|value| value.trim_end_matches('%').parse().ok())
}

fn validate_container_id(container_id: &str) -> AppResult<()> {
    if container_id.len() != 64 || !container_id.chars().all(|value| value.is_ascii_hexdigit()) {
        return Err(AppError::Message("Docker 容器 ID 无效".into()));
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
