use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;
use ssh2::Session;

use crate::error::{AppError, AppResult};
use crate::models::{
    ComposeAction, ComposeActionResult, ComposeLogs, ComposeOverview, ComposeProject, DockerAction,
    DockerActionResult, DockerContainer, DockerLogs, DockerOverview, DockerService,
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

pub fn compose_list(session: &Session) -> AppResult<ComposeOverview> {
    let version = execute_compose_checked(session, "docker compose version --short")?;
    Ok(ComposeOverview {
        version: version.stdout.trim().to_string(),
        projects: compose_projects(session)?,
    })
}

pub fn compose_action(
    session: &Session,
    project_name: &str,
    action: ComposeAction,
) -> AppResult<ComposeActionResult> {
    let project = find_compose_project(session, project_name)?;
    let action_name = match action {
        ComposeAction::Up => "up -d",
        ComposeAction::Down => "down",
        ComposeAction::Restart => "restart",
        ComposeAction::Pull => "pull",
    };
    let command = format!("{} {action_name}", compose_command(&project));
    let output = execute_compose_checked(session, &command)?;
    Ok(ComposeActionResult {
        project: project_name.to_string(),
        action,
        output: combined_output(&output),
        exit_code: output.exit_code,
    })
}

pub fn compose_logs(session: &Session, project_name: &str) -> AppResult<ComposeLogs> {
    let project = find_compose_project(session, project_name)?;
    let command = format!("{} logs --tail 200 --no-color", compose_command(&project));
    let output = execute_compose_checked(session, &command)?;
    Ok(ComposeLogs {
        project: project_name.to_string(),
        output: combined_output(&output),
    })
}

fn compose_projects(session: &Session) -> AppResult<Vec<ComposeProject>> {
    let listing = execute_compose_checked(session, "docker compose ls -a --format json")?;
    let trimmed = listing.stdout.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let values = if trimmed.starts_with('[') {
        serde_json::from_str::<Vec<Value>>(trimmed).map_err(|error| {
            AppError::Message(format!("无法解析 Docker Compose 项目列表：{error}"))
        })?
    } else {
        trimmed
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str::<Value>(line).map_err(|error| {
                    AppError::Message(format!("无法解析 Docker Compose 项目列表：{error}"))
                })
            })
            .collect::<AppResult<Vec<_>>>()?
    };
    let mut projects = values
        .iter()
        .map(|value| {
            let name = value
                .get("Name")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::Message("Docker Compose 项目缺少名称".into()))?;
            let config_files = value
                .get("ConfigFiles")
                .and_then(Value::as_str)
                .unwrap_or("")
                .split(',')
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            if config_files.is_empty() {
                return Err(AppError::Message(format!(
                    "Compose 项目 {name} 没有配置文件路径"
                )));
            }
            let working_directory = Path::new(&config_files[0])
                .parent()
                .ok_or_else(|| AppError::Message(format!("Compose 项目 {name} 配置路径无效")))?
                .to_string_lossy()
                .into_owned();
            Ok(ComposeProject {
                name: name.to_string(),
                status: value
                    .get("Status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                config_files,
                working_directory,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
    projects.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(projects)
}

pub(crate) fn find_compose_project(
    session: &Session,
    project_name: &str,
) -> AppResult<ComposeProject> {
    if project_name.is_empty()
        || project_name.len() > 128
        || !project_name
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '-' | '_' | '.'))
    {
        return Err(AppError::Message("Docker Compose 项目名称无效".into()));
    }
    compose_projects(session)?
        .into_iter()
        .find(|project| project.name == project_name)
        .ok_or_else(|| AppError::Message("Compose 项目已不存在，请刷新列表".into()))
}

fn compose_command(project: &ComposeProject) -> String {
    let files = project
        .config_files
        .iter()
        .map(|path| format!("-f {}", shell_quote(path)))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "docker compose {files} --project-name {}",
        shell_quote(&project.name)
    )
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn execute_compose_checked(session: &Session, command: &str) -> AppResult<ExecOutput> {
    let output = ssh::execute(session, command)?;
    if output.exit_code != 0 {
        return Err(AppError::Message(format!(
            "无法访问 Docker Compose v2，请确认 compose 插件已安装且当前用户有权限：{}",
            combined_output(&output).trim()
        )));
    }
    Ok(output)
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
    let id = required("ID")?;
    let name = required("Names")?;
    let ports = value
        .get("Ports")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let services = parse_services(&id, &name, &ports);
    Ok(DockerContainer {
        id,
        name,
        image: required("Image")?,
        state: required("State")?,
        status: required("Status")?,
        ports,
        services,
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

fn parse_services(container_id: &str, container_name: &str, ports: &str) -> Vec<DockerService> {
    let mut services = ports
        .split(',')
        .filter_map(|entry| {
            let (published, target) = entry.trim().split_once("->")?;
            let (_, host_port) = published.rsplit_once(':')?;
            let (container_port, protocol) = target.split_once('/')?;
            Some(DockerService {
                container_id: container_id.to_string(),
                container_name: container_name.to_string(),
                host_port: host_port.parse().ok()?,
                container_port: container_port.parse().ok()?,
                protocol: protocol.to_string(),
            })
        })
        .collect::<Vec<_>>();
    services.sort_by_key(|service| {
        (
            service.host_port,
            service.container_port,
            service.protocol.clone(),
        )
    });
    services.dedup_by_key(|service| {
        (
            service.host_port,
            service.container_port,
            service.protocol.clone(),
        )
    });
    services
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
