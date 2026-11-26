use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use ssh2::{FileStat, RenameFlags, Session, Sftp};
use uuid::Uuid;

use crate::docker;
use crate::error::{AppError, AppResult};
use crate::models::{
    ComposeApplication, ComposeDeploymentAction, ComposeDeploymentHistory, ComposeDeploymentPlan,
    ComposeDeploymentRecord, ComposeDeploymentResult, ComposeDeploymentStatus, ComposeEditInput,
    ComposeEnvironmentChange, ComposeEnvironmentEntry, ComposeImageSnapshot, ComposeProject,
    ComposeServiceChange,
};
use crate::ssh::{self, ExecOutput};

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_ENV_BYTES: u64 = 256 * 1024;
const MAX_METADATA_BYTES: u64 = 512 * 1024;
const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_CONFIG_FILES: usize = 8;
const MAX_HISTORY_RECORDS: usize = 50;
const HISTORY_DIRECTORY: &str = ".controlvps/deployments";

#[derive(Debug, Clone)]
struct ComposeTarget {
    name: String,
    working_directory: PathBuf,
    config_files: Vec<PathBuf>,
    environment_path: PathBuf,
}

#[derive(Debug, Clone)]
struct CurrentConfiguration {
    files: Vec<(PathBuf, Vec<u8>)>,
    environment: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
struct ContainerState {
    service: String,
    image: String,
    image_id: Option<String>,
    config_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredFile {
    original_path: String,
    storage_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSnapshot {
    files: Vec<StoredFile>,
    environment: Option<StoredFile>,
    images: Vec<ComposeImageSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredDeployment {
    record: ComposeDeploymentRecord,
    snapshot: Option<StoredSnapshot>,
}

struct DeploymentLock {
    sftp: Sftp,
    path: PathBuf,
}

impl Drop for DeploymentLock {
    fn drop(&mut self) {
        let _ = self.sftp.rmdir(&self.path);
    }
}

pub fn read(
    session: &Session,
    server_id: &str,
    project_name: &str,
) -> AppResult<ComposeApplication> {
    let project = docker::find_compose_project(session, project_name)?;
    let target = target_from_project(&project)?;
    let current = read_current(session, &target)?
        .ok_or_else(|| AppError::Message("Compose 主配置文件不存在，请刷新项目列表".into()))?;
    let content = String::from_utf8(current.files[0].1.clone())
        .map_err(|_| AppError::Message("Compose 主配置不是有效的 UTF-8 文本".into()))?;
    let remote_environment = current
        .environment
        .as_deref()
        .and_then(|value| parse_environment(value).ok())
        .unwrap_or_default();
    let managed = managed_environment(server_id, project_name)?.unwrap_or_default();
    let keys = remote_environment
        .keys()
        .chain(managed.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let environment = keys
        .into_iter()
        .map(|key| ComposeEnvironmentEntry {
            managed: managed.contains_key(&key),
            key,
        })
        .collect();
    Ok(ComposeApplication {
        revision: configuration_revision(&current),
        project,
        content,
        environment,
    })
}

pub fn preview(session: &Session, input: ComposeEditInput) -> AppResult<ComposeDeploymentPlan> {
    validate_edit_input(&input)?;
    let target = target_from_input(session, &input)?;
    let current = read_current(session, &target)?;
    ensure_expected_revision(current.as_ref(), &input.expected_revision)?;
    let desired_files = desired_files(&input, &target, current.as_ref())?;
    let desired_environment = merged_environment(current.as_ref(), &input.environment_changes)?;
    build_plan(
        session,
        &target,
        desired_files,
        desired_environment,
        current.as_ref(),
    )
}

pub fn deploy(
    session: &Session,
    server_id: &str,
    input: ComposeEditInput,
) -> AppResult<ComposeDeploymentResult> {
    validate_edit_input(&input)?;
    let target = target_from_input(session, &input)?;
    let _lock = acquire_lock(session, &target)?;
    let current = read_current(session, &target)?;
    ensure_expected_revision(current.as_ref(), &input.expected_revision)?;
    let desired_files = desired_files(&input, &target, current.as_ref())?;
    let desired_environment = merged_environment(current.as_ref(), &input.environment_changes)?;
    let plan = build_plan(
        session,
        &target,
        desired_files.clone(),
        desired_environment.clone(),
        current.as_ref(),
    )?;
    let changed_files = vec![desired_files[0].clone()];
    execute_deployment(
        session,
        server_id,
        &target,
        ComposeDeploymentAction::Deploy,
        Uuid::new_v4().to_string(),
        current,
        changed_files,
        target.config_files.clone(),
        desired_environment,
        !input.environment_changes.is_empty(),
        plan,
    )
}

pub fn history(session: &Session, project_name: &str) -> AppResult<ComposeDeploymentHistory> {
    let project = docker::find_compose_project(session, project_name)?;
    let target = target_from_project(&project)?;
    let current_revision = read_current(session, &target)?
        .as_ref()
        .map(configuration_revision)
        .unwrap_or_default();
    let root = target.working_directory.join(HISTORY_DIRECTORY);
    let sftp = session.sftp()?;
    let mut records = match sftp.readdir(&root) {
        Ok(entries) => entries
            .into_iter()
            .filter(|(path, stat)| {
                stat.is_dir()
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| Uuid::parse_str(name).is_ok())
            })
            .filter_map(|(path, _)| read_stored_deployment(&sftp, &path).ok())
            .filter(|stored| stored.record.project == project_name)
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    records.sort_by(|left, right| right.record.created_at.cmp(&left.record.created_at));
    records.truncate(MAX_HISTORY_RECORDS);
    Ok(ComposeDeploymentHistory {
        project: project_name.to_string(),
        current_revision,
        records: records.into_iter().map(|stored| stored.record).collect(),
    })
}

pub fn rollback(
    session: &Session,
    server_id: &str,
    project_name: &str,
    deployment_id: &str,
    expected_revision: &str,
) -> AppResult<ComposeDeploymentResult> {
    Uuid::parse_str(deployment_id).map_err(|_| AppError::Message("部署历史 ID 无效".into()))?;
    let project = docker::find_compose_project(session, project_name)?;
    let target = target_from_project(&project)?;
    let _lock = acquire_lock(session, &target)?;
    let current = read_current(session, &target)?;
    ensure_expected_revision(current.as_ref(), expected_revision)?;
    let sftp = session.sftp()?;
    let stored = read_stored_deployment(
        &sftp,
        &target
            .working_directory
            .join(HISTORY_DIRECTORY)
            .join(deployment_id),
    )?;
    let snapshot = stored
        .snapshot
        .ok_or_else(|| AppError::Message("首次部署没有上一版配置，无法执行回滚".into()))?;
    let expected_paths = target
        .config_files
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<BTreeSet<_>>();
    let snapshot_paths = snapshot
        .files
        .iter()
        .map(|file| file.original_path.clone())
        .collect::<BTreeSet<_>>();
    if snapshot_paths != expected_paths {
        return Err(AppError::Message(
            "项目的 Compose 文件列表已变化，为避免覆盖配置，不能自动回滚此快照".into(),
        ));
    }
    if snapshot
        .environment
        .as_ref()
        .is_some_and(|file| Path::new(&file.original_path) != target.environment_path)
    {
        return Err(AppError::Message("快照中的环境变量文件路径无效".into()));
    }

    let record_id = Uuid::new_v4().to_string();
    let override_path = target
        .working_directory
        .join(".controlvps")
        .join(format!("rollback-{record_id}.yaml"));
    let override_content = image_override(&snapshot.images)?;
    let mut desired_files = snapshot
        .files
        .iter()
        .map(|file| {
            validate_storage_name(&file.storage_name)?;
            let content = read_required(
                &sftp,
                &stored_snapshot_path(&target, deployment_id, &file.storage_name),
                MAX_CONFIG_BYTES,
            )?;
            Ok((PathBuf::from(&file.original_path), content))
        })
        .collect::<AppResult<Vec<_>>>()?;
    if let Some(content) = override_content {
        desired_files.push((override_path, content));
    }
    let desired_environment = snapshot
        .environment
        .as_ref()
        .map(|file| {
            validate_storage_name(&file.storage_name)?;
            read_required(
                &sftp,
                &stored_snapshot_path(&target, deployment_id, &file.storage_name),
                MAX_ENV_BYTES,
            )
        })
        .transpose()?;
    let plan = build_plan(
        session,
        &target,
        desired_files.clone(),
        desired_environment.clone(),
        current.as_ref(),
    )?;
    let command_files = desired_files.iter().map(|(path, _)| path.clone()).collect();
    execute_deployment(
        session,
        server_id,
        &target,
        ComposeDeploymentAction::Rollback,
        record_id,
        current,
        desired_files,
        command_files,
        desired_environment,
        true,
        plan,
    )
}

pub fn redact_project_output(
    session: &Session,
    server_id: &str,
    project_name: &str,
    output: &str,
) -> AppResult<String> {
    let project = docker::find_compose_project(session, project_name)?;
    let target = target_from_project(&project)?;
    let remote = read_current(session, &target)?
        .and_then(|current| current.environment)
        .map(|environment| parse_environment(&environment))
        .transpose()?
        .unwrap_or_default();
    let values = remote
        .into_values()
        .chain(
            managed_environment(server_id, project_name)?
                .unwrap_or_default()
                .into_values(),
        )
        .collect::<Vec<_>>();
    Ok(redact(output, values))
}

fn validate_edit_input(input: &ComposeEditInput) -> AppResult<()> {
    validate_project_name(&input.project_name)?;
    if input.content.as_bytes().len() as u64 > MAX_CONFIG_BYTES {
        return Err(AppError::Message("Compose YAML 不能超过 1 MiB".into()));
    }
    if input.content.trim().is_empty() {
        return Err(AppError::Message("Compose YAML 不能为空".into()));
    }
    if input.config_files.len() > MAX_CONFIG_FILES {
        return Err(AppError::Message(
            "一个项目最多支持 8 个 Compose 文件".into(),
        ));
    }
    for change in &input.environment_changes {
        validate_environment_key(&change.key)?;
        if change
            .value
            .as_ref()
            .is_some_and(|value| value.contains(['\0', '\r', '\n']) || value.len() > 64 * 1024)
        {
            return Err(AppError::Message(format!(
                "环境变量 {} 的值不能包含换行或 NUL，且不能超过 64 KiB",
                change.key
            )));
        }
    }
    Ok(())
}

fn target_from_input(session: &Session, input: &ComposeEditInput) -> AppResult<ComposeTarget> {
    let working_directory = resolve_working_directory(session, &input.working_directory, true)?;
    let config_files = if input.config_files.is_empty() {
        vec![working_directory.join("compose.yaml")]
    } else {
        input
            .config_files
            .iter()
            .map(|path| safe_remote_file(path))
            .collect::<AppResult<Vec<_>>>()?
    };
    validate_config_files(&working_directory, &config_files)?;
    Ok(ComposeTarget {
        name: input.project_name.clone(),
        environment_path: working_directory.join(".env"),
        working_directory,
        config_files,
    })
}

fn target_from_project(project: &ComposeProject) -> AppResult<ComposeTarget> {
    validate_project_name(&project.name)?;
    let working_directory = safe_remote_directory(&project.working_directory)?;
    let config_files = project
        .config_files
        .iter()
        .filter(|path| !is_generated_override(path))
        .map(|path| safe_remote_file(path))
        .collect::<AppResult<Vec<_>>>()?;
    validate_config_files(&working_directory, &config_files)?;
    Ok(ComposeTarget {
        name: project.name.clone(),
        environment_path: working_directory.join(".env"),
        working_directory,
        config_files,
    })
}

fn validate_config_files(working_directory: &Path, files: &[PathBuf]) -> AppResult<()> {
    if files.is_empty() || files.len() > MAX_CONFIG_FILES {
        return Err(AppError::Message("Compose 配置文件数量无效".into()));
    }
    if files[0].parent() != Some(working_directory) {
        return Err(AppError::Message(
            "Compose 主配置必须位于项目工作目录中".into(),
        ));
    }
    Ok(())
}

fn validate_project_name(name: &str) -> AppResult<()> {
    if name.is_empty()
        || name.len() > 128
        || !name
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '-' | '_' | '.'))
    {
        return Err(AppError::Message("Docker Compose 项目名称无效".into()));
    }
    Ok(())
}

fn resolve_working_directory(session: &Session, value: &str, create: bool) -> AppResult<PathBuf> {
    let expanded = if value == "~" || value.starts_with("~/") {
        let output = ssh::execute(session, "printf '%s' \"$HOME\"")?;
        if output.exit_code != 0 || output.stdout.trim().is_empty() {
            return Err(AppError::Message("无法解析远程用户的主目录".into()));
        }
        PathBuf::from(output.stdout.trim()).join(value.strip_prefix("~/").unwrap_or(""))
    } else {
        PathBuf::from(value)
    };
    let path = safe_remote_directory(&expanded.to_string_lossy())?;
    let sftp = session.sftp()?;
    if create {
        ensure_directory(&sftp, &path, 0o755)?;
    }
    sftp.realpath(&path)
        .map_err(|error| AppError::Message(format!("无法访问 Compose 工作目录：{error}")))
}

fn safe_remote_directory(value: &str) -> AppResult<PathBuf> {
    let path = PathBuf::from(value.trim());
    if value.trim().is_empty()
        || value.contains('\0')
        || !path.is_absolute()
        || path.parent().is_none()
        || value.split('/').any(|part| part == "..")
    {
        return Err(AppError::Message(
            "项目目录必须是根目录之外的绝对路径，且不能包含 ..".into(),
        ));
    }
    Ok(path)
}

fn safe_remote_file(value: &str) -> AppResult<PathBuf> {
    let path = safe_remote_directory(value)?;
    if path.file_name().is_none() {
        return Err(AppError::Message("Compose 配置文件路径无效".into()));
    }
    Ok(path)
}

fn is_generated_override(path: &str) -> bool {
    let path = Path::new(path);
    path.parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == ".controlvps")
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("rollback-") && name.ends_with(".yaml"))
}

fn read_current(
    session: &Session,
    target: &ComposeTarget,
) -> AppResult<Option<CurrentConfiguration>> {
    let sftp = session.sftp()?;
    let Some(primary) = read_optional(&sftp, &target.config_files[0], MAX_CONFIG_BYTES)? else {
        if read_optional(&sftp, &target.environment_path, MAX_ENV_BYTES)?.is_some() {
            return Err(AppError::Message(
                "项目目录已有 .env 但没有 Compose 主配置，为避免覆盖，请先手工检查目录".into(),
            ));
        }
        return Ok(None);
    };
    let mut files = vec![(target.config_files[0].clone(), primary)];
    for path in target.config_files.iter().skip(1) {
        files.push((path.clone(), read_required(&sftp, path, MAX_CONFIG_BYTES)?));
    }
    let environment = read_optional(&sftp, &target.environment_path, MAX_ENV_BYTES)?;
    Ok(Some(CurrentConfiguration { files, environment }))
}

fn desired_files(
    input: &ComposeEditInput,
    target: &ComposeTarget,
    current: Option<&CurrentConfiguration>,
) -> AppResult<Vec<(PathBuf, Vec<u8>)>> {
    let mut files = current
        .map(|current| current.files.clone())
        .unwrap_or_else(|| {
            target
                .config_files
                .iter()
                .map(|path| (path.clone(), Vec::new()))
                .collect()
        });
    files[0].1 = input.content.as_bytes().to_vec();
    Ok(files)
}

fn ensure_expected_revision(
    current: Option<&CurrentConfiguration>,
    expected: &str,
) -> AppResult<()> {
    let actual = current.map(configuration_revision).unwrap_or_default();
    if actual == expected {
        return Ok(());
    }
    if expected.is_empty() && current.is_some() {
        Err(AppError::Message(
            "该目录已经存在 Compose 配置，请改为从现有项目打开发布编辑器".into(),
        ))
    } else {
        Err(AppError::Message(
            "配置在预检后已被其他操作修改，请重新打开编辑器并复核变更".into(),
        ))
    }
}

fn configuration_revision(configuration: &CurrentConfiguration) -> String {
    let mut hash = Sha256::new();
    for (path, content) in &configuration.files {
        hash.update(path.to_string_lossy().as_bytes());
        hash.update((content.len() as u64).to_be_bytes());
        hash.update(content);
    }
    if let Some(environment) = &configuration.environment {
        hash.update((environment.len() as u64).to_be_bytes());
        hash.update(environment);
    }
    format!("sha256:{:x}", hash.finalize())
}

fn build_plan(
    session: &Session,
    target: &ComposeTarget,
    desired_files: Vec<(PathBuf, Vec<u8>)>,
    desired_environment: Option<Vec<u8>>,
    current: Option<&CurrentConfiguration>,
) -> AppResult<ComposeDeploymentPlan> {
    let sftp = session.sftp()?;
    let mut staged_files = Vec::new();
    let mut staged_environment = None;
    let result = (|| {
        for (path, content) in &desired_files {
            let parent = path
                .parent()
                .ok_or_else(|| AppError::Message("Compose 配置路径无效".into()))?;
            ensure_directory(&sftp, parent, 0o755)?;
            let staged = parent.join(format!(".controlvps-preview-{}.yaml", Uuid::new_v4()));
            atomic_write(&sftp, &staged, content, 0o600)?;
            staged_files.push(staged);
        }
        if let Some(environment) = &desired_environment {
            let staged = target
                .working_directory
                .join(format!(".controlvps-preview-{}.env", Uuid::new_v4()));
            atomic_write(&sftp, &staged, environment, 0o600)?;
            staged_environment = Some(staged);
        }
        let secrets = desired_environment
            .as_deref()
            .map(parse_environment)
            .transpose()?
            .unwrap_or_default()
            .into_values()
            .collect::<Vec<_>>();
        let command = compose_command(target, &staged_files, staged_environment.as_deref());
        let validation = ssh::execute(session, &format!("{command} config --quiet"))?;
        if validation.exit_code != 0 {
            return Err(AppError::Message(format!(
                "Compose 服务端校验失败：{}",
                redact(&combined_output(&validation), secrets.clone()).trim()
            )));
        }
        let hashes = ssh::execute(session, &format!("{command} config --hash '*'"))?;
        if hashes.exit_code != 0 {
            return Err(AppError::Message(format!(
                "无法计算 Compose 服务配置指纹：{}",
                redact(&combined_output(&hashes), secrets.clone()).trim()
            )));
        }
        let rendered = ssh::execute(session, &format!("{command} config --format json"))?;
        if rendered.exit_code != 0 {
            return Err(AppError::Message(format!(
                "无法解析 Compose 服务配置：{}",
                redact(&combined_output(&rendered), secrets).trim()
            )));
        }
        let desired = desired_services(target, &hashes.stdout, &rendered.stdout)?;
        let current_services = if let Some(current) = current {
            current_state(
                session,
                target,
                &target.config_files,
                current
                    .environment
                    .as_ref()
                    .map(|_| target.environment_path.as_path()),
            )?
        } else {
            Vec::new()
        };
        Ok(make_plan(
            target,
            current.map(configuration_revision).unwrap_or_default(),
            desired,
            current_services,
        ))
    })();
    for path in staged_files {
        let _ = sftp.unlink(&path);
    }
    if let Some(path) = staged_environment {
        let _ = sftp.unlink(&path);
    }
    result
}

fn desired_services(
    target: &ComposeTarget,
    hashes: &str,
    rendered: &str,
) -> AppResult<BTreeMap<String, (String, Option<String>)>> {
    let mut result = BTreeMap::new();
    for line in hashes.lines().filter(|line| !line.trim().is_empty()) {
        let parts = line.split_whitespace().collect::<Vec<_>>();
        if parts.len() < 2 {
            return Err(AppError::Message("Compose 返回了无法识别的服务指纹".into()));
        }
        result.insert(
            parts[0].to_string(),
            (parts[parts.len() - 1].to_string(), None),
        );
    }
    let value: Value = serde_json::from_str(rendered)
        .map_err(|error| AppError::Message(format!("无法解析 Compose 规范化配置：{error}")))?;
    let services = value
        .get("services")
        .and_then(Value::as_object)
        .ok_or_else(|| AppError::Message("Compose 配置中没有 services".into()))?;
    for (service, definition) in services {
        let image = definition
            .get("image")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("{}-{}", target.name, service));
        result
            .entry(service.clone())
            .or_insert_with(|| (String::new(), None))
            .1 = Some(image);
    }
    if result.is_empty() {
        return Err(AppError::Message("Compose 配置至少需要一个服务".into()));
    }
    Ok(result)
}

fn make_plan(
    target: &ComposeTarget,
    revision: String,
    desired: BTreeMap<String, (String, Option<String>)>,
    current: Vec<ContainerState>,
) -> ComposeDeploymentPlan {
    let current_by_service = current
        .into_iter()
        .map(|state| (state.service.clone(), state))
        .collect::<HashMap<_, _>>();
    let mut services = desired
        .iter()
        .map(|(service, (desired_hash, desired_image))| {
            let existing = current_by_service.get(service);
            let change = match existing {
                None => "create",
                Some(state) if state.config_hash.as_deref() != Some(desired_hash) => "update",
                Some(_) => "unchanged",
            };
            ComposeServiceChange {
                service: service.clone(),
                change: change.to_string(),
                current_image: existing.map(|state| state.image.clone()),
                desired_image: desired_image.clone(),
                current_image_id: existing.and_then(|state| state.image_id.clone()),
            }
        })
        .collect::<Vec<_>>();
    services.extend(current_by_service.iter().filter_map(|(service, state)| {
        if desired.contains_key(service) {
            return None;
        }
        Some(ComposeServiceChange {
            service: service.clone(),
            change: "remove".into(),
            current_image: Some(state.image.clone()),
            desired_image: None,
            current_image_id: state.image_id.clone(),
        })
    }));
    services.sort_by(|left, right| left.service.cmp(&right.service));
    let count = |kind: &str| {
        services
            .iter()
            .filter(|service| service.change == kind)
            .count()
    };
    ComposeDeploymentPlan {
        project: target.name.clone(),
        revision,
        create_count: count("create"),
        update_count: count("update"),
        remove_count: count("remove"),
        unchanged_count: count("unchanged"),
        services,
    }
}

fn current_state(
    session: &Session,
    target: &ComposeTarget,
    config_files: &[PathBuf],
    environment_path: Option<&Path>,
) -> AppResult<Vec<ContainerState>> {
    let command = compose_command(target, config_files, environment_path);
    let listing = ssh::execute(session, &format!("{command} ps -a --format json"))?;
    if listing.exit_code != 0 {
        return Err(AppError::Message(format!(
            "无法读取 Compose 当前服务：{}",
            combined_output(&listing).trim()
        )));
    }
    let values = parse_json_values(&listing.stdout)?;
    let mut states = Vec::new();
    let mut ids = Vec::new();
    for value in values {
        let service = value
            .get("Service")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let id = value.get("ID").and_then(Value::as_str).unwrap_or("");
        if service.is_empty()
            || id.len() < 12
            || !id.chars().all(|character| character.is_ascii_hexdigit())
        {
            continue;
        }
        ids.push(id.to_string());
        states.push(ContainerState {
            service,
            image: value
                .get("Image")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            image_id: None,
            config_hash: None,
        });
    }
    if ids.is_empty() {
        return Ok(states);
    }
    let inspect = ssh::execute(
        session,
        &format!(
            "docker inspect --format '{{{{index .Config.Labels \"com.docker.compose.config-hash\"}}}}|{{{{.Config.Image}}}}|{{{{.Image}}}}' {}",
            ids.join(" ")
        ),
    )?;
    if inspect.exit_code != 0 {
        return Err(AppError::Message(format!(
            "无法读取 Compose 容器镜像摘要：{}",
            combined_output(&inspect).trim()
        )));
    }
    for (state, line) in states.iter_mut().zip(inspect.stdout.lines()) {
        let mut parts = line.splitn(3, '|');
        let hash = parts.next().unwrap_or("");
        let image = parts.next().unwrap_or("");
        let image_id = parts.next().unwrap_or("");
        state.config_hash = (!hash.is_empty() && hash != "<no value>").then(|| hash.to_string());
        if !image.is_empty() {
            state.image = image.to_string();
        }
        state.image_id = valid_image_id(image_id).then(|| image_id.to_string());
    }
    Ok(states)
}

fn parse_json_values(output: &str) -> AppResult<Vec<Value>> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if trimmed.starts_with('[') {
        serde_json::from_str(trimmed)
            .map_err(|error| AppError::Message(format!("无法解析 Compose 服务列表：{error}")))
    } else {
        trimmed
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str(line).map_err(|error| {
                    AppError::Message(format!("无法解析 Compose 服务列表：{error}"))
                })
            })
            .collect()
    }
}

fn compose_command(
    target: &ComposeTarget,
    config_files: &[PathBuf],
    environment_path: Option<&Path>,
) -> String {
    let files = config_files
        .iter()
        .map(|path| format!("-f {}", shell_quote(&path.to_string_lossy())))
        .collect::<Vec<_>>()
        .join(" ");
    let environment = environment_path
        .map(|path| format!(" --env-file {}", shell_quote(&path.to_string_lossy())))
        .unwrap_or_default();
    format!(
        "docker compose --project-name {} --project-directory {} {files}{environment}",
        shell_quote(&target.name),
        shell_quote(&target.working_directory.to_string_lossy()),
    )
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn combined_output(output: &ExecOutput) -> String {
    let mut text = output.stdout.clone();
    if !output.stderr.trim().is_empty() {
        text.push_str(&output.stderr);
    }
    text
}

#[allow(clippy::too_many_arguments)]
fn execute_deployment(
    session: &Session,
    server_id: &str,
    target: &ComposeTarget,
    action: ComposeDeploymentAction,
    record_id: String,
    current: Option<CurrentConfiguration>,
    write_files: Vec<(PathBuf, Vec<u8>)>,
    command_files: Vec<PathBuf>,
    desired_environment: Option<Vec<u8>>,
    update_managed_environment: bool,
    plan: ComposeDeploymentPlan,
) -> AppResult<ComposeDeploymentResult> {
    let sftp = session.sftp()?;
    let snapshot_images = if current.is_some() {
        current_state(
            session,
            target,
            &target.config_files,
            current
                .as_ref()
                .and_then(|current| current.environment.as_ref())
                .map(|_| target.environment_path.as_path()),
        )?
        .into_iter()
        .map(|state| ComposeImageSnapshot {
            service: state.service,
            image: state.image,
            image_id: state.image_id,
        })
        .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let snapshot = create_snapshot(&sftp, target, &record_id, current.as_ref(), snapshot_images)?;
    let revision_before = current.as_ref().map(configuration_revision);
    let mut record = ComposeDeploymentRecord {
        id: record_id.clone(),
        project: target.name.clone(),
        action,
        status: ComposeDeploymentStatus::Failed,
        created_at: Utc::now().to_rfc3339(),
        operator: std::env::var("USER").unwrap_or_else(|_| "local".into()),
        snapshot_available: snapshot.is_some(),
        snapshot_image_count: snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .images
                    .iter()
                    .filter(|image| image.image_id.is_some())
                    .count()
            })
            .unwrap_or(0),
        revision_before,
        revision_after: None,
        changes: plan.services.clone(),
        images: Vec::new(),
        output: "部署尚未完成".into(),
    };
    save_stored_deployment(
        &sftp,
        target,
        &StoredDeployment {
            record: record.clone(),
            snapshot: snapshot.clone(),
        },
    )?;

    let mut secret_values = BTreeMap::new();
    if let Some(environment) = current
        .as_ref()
        .and_then(|current| current.environment.as_deref())
    {
        secret_values.extend(parse_environment(environment)?);
    }
    if let Some(environment) = desired_environment.as_deref() {
        secret_values.extend(parse_environment(environment)?);
    }
    let secrets = secret_values.into_values().collect::<Vec<_>>();
    let outcome = (|| {
        for (path, content) in &write_files {
            if let Some(parent) = path.parent() {
                ensure_directory(&sftp, parent, 0o700)?;
            }
            atomic_write(&sftp, path, content, 0o600)?;
        }
        match &desired_environment {
            Some(environment) => atomic_write(&sftp, &target.environment_path, environment, 0o600)?,
            None => remove_optional(&sftp, &target.environment_path)?,
        }
        if update_managed_environment {
            let environment = desired_environment
                .as_deref()
                .map(parse_environment)
                .transpose()?;
            save_managed_environment(server_id, &target.name, environment)?;
        }
        let command = compose_command(
            target,
            &command_files,
            desired_environment
                .as_ref()
                .map(|_| target.environment_path.as_path()),
        );
        ssh::execute(session, &format!("{command} up -d --remove-orphans"))
    })();

    match outcome {
        Ok(output) => {
            record.status = if output.exit_code == 0 {
                ComposeDeploymentStatus::Succeeded
            } else {
                ComposeDeploymentStatus::Failed
            };
            record.output = truncate_output(&redact(&combined_output(&output), secrets));
        }
        Err(error) => {
            record.output = truncate_output(&redact(&error.to_string(), secrets));
            record.revision_after = read_current(session, target)
                .ok()
                .flatten()
                .as_ref()
                .map(configuration_revision);
            let _ = save_stored_deployment(
                &sftp,
                target,
                &StoredDeployment {
                    record: record.clone(),
                    snapshot,
                },
            );
            return Err(error);
        }
    }
    record.revision_after = read_current(session, target)
        .map_err(|error| AppError::Message(format!("读取部署后的 Compose 配置失败：{error}")))?
        .as_ref()
        .map(configuration_revision);
    record.images = current_state(
        session,
        target,
        &command_files,
        desired_environment
            .as_ref()
            .map(|_| target.environment_path.as_path()),
    )
    .unwrap_or_default()
    .into_iter()
    .map(|state| ComposeImageSnapshot {
        service: state.service,
        image: state.image,
        image_id: state.image_id,
    })
    .collect();
    save_stored_deployment(
        &sftp,
        target,
        &StoredDeployment {
            record: record.clone(),
            snapshot,
        },
    )
    .map_err(|error| AppError::Message(format!("更新部署历史失败：{error}")))?;
    Ok(ComposeDeploymentResult { record, plan })
}

fn create_snapshot(
    sftp: &Sftp,
    target: &ComposeTarget,
    record_id: &str,
    current: Option<&CurrentConfiguration>,
    images: Vec<ComposeImageSnapshot>,
) -> AppResult<Option<StoredSnapshot>> {
    let directory = target
        .working_directory
        .join(HISTORY_DIRECTORY)
        .join(record_id);
    ensure_directory(sftp, &directory, 0o700)?;
    let Some(current) = current else {
        return Ok(None);
    };
    let files_directory = directory.join("files");
    ensure_directory(sftp, &files_directory, 0o700)?;
    let mut files = Vec::new();
    for (index, (path, content)) in current.files.iter().enumerate() {
        let storage_name = format!("config-{index}");
        atomic_write(sftp, &files_directory.join(&storage_name), content, 0o600)?;
        files.push(StoredFile {
            original_path: path.to_string_lossy().into_owned(),
            storage_name,
        });
    }
    let environment = current
        .environment
        .as_ref()
        .map(|content| {
            let storage_name = "environment".to_string();
            atomic_write(sftp, &files_directory.join(&storage_name), content, 0o600)?;
            Ok::<StoredFile, AppError>(StoredFile {
                original_path: target.environment_path.to_string_lossy().into_owned(),
                storage_name,
            })
        })
        .transpose()?;
    Ok(Some(StoredSnapshot {
        files,
        environment,
        images,
    }))
}

fn save_stored_deployment(
    sftp: &Sftp,
    target: &ComposeTarget,
    stored: &StoredDeployment,
) -> AppResult<()> {
    let path = target
        .working_directory
        .join(HISTORY_DIRECTORY)
        .join(&stored.record.id)
        .join("metadata.json");
    let content = serde_json::to_vec_pretty(stored)
        .map_err(|error| AppError::Message(format!("无法保存部署历史：{error}")))?;
    atomic_write(sftp, &path, &content, 0o600)
}

fn read_stored_deployment(sftp: &Sftp, directory: &Path) -> AppResult<StoredDeployment> {
    let content = read_required(sftp, &directory.join("metadata.json"), MAX_METADATA_BYTES)?;
    serde_json::from_slice(&content)
        .map_err(|error| AppError::Message(format!("无法读取部署历史：{error}")))
}

fn stored_snapshot_path(target: &ComposeTarget, record_id: &str, storage_name: &str) -> PathBuf {
    target
        .working_directory
        .join(HISTORY_DIRECTORY)
        .join(record_id)
        .join("files")
        .join(storage_name)
}

fn validate_storage_name(name: &str) -> AppResult<()> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return Err(AppError::Message("历史快照中的存储文件名无效".into()));
    }
    Ok(())
}

fn image_override(images: &[ComposeImageSnapshot]) -> AppResult<Option<Vec<u8>>> {
    let pinned = images
        .iter()
        .filter(|image| image.image_id.as_deref().is_some_and(valid_image_id))
        .collect::<Vec<_>>();
    if pinned.is_empty() {
        return Ok(None);
    }
    let mut output = String::from("services:\n");
    for image in pinned {
        if !image.service.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        }) {
            return Err(AppError::Message("历史快照中的 Compose 服务名无效".into()));
        }
        output.push_str(&format!(
            "  {}:\n    image: \"{}\"\n",
            image.service,
            image.image_id.as_deref().unwrap_or_default()
        ));
    }
    Ok(Some(output.into_bytes()))
}

fn valid_image_id(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

fn merged_environment(
    current: Option<&CurrentConfiguration>,
    changes: &[ComposeEnvironmentChange],
) -> AppResult<Option<Vec<u8>>> {
    let existing = current.and_then(|current| current.environment.clone());
    if changes.is_empty() {
        return Ok(existing);
    }
    let mut values = existing
        .as_deref()
        .map(parse_environment)
        .transpose()?
        .unwrap_or_default();
    for change in changes {
        match &change.value {
            Some(value) => {
                values.insert(change.key.clone(), value.clone());
            }
            None => {
                values.remove(&change.key);
            }
        }
    }
    Ok(Some(serialize_environment(&values)))
}

fn parse_environment(content: &[u8]) -> AppResult<BTreeMap<String, String>> {
    let content = std::str::from_utf8(content)
        .map_err(|_| AppError::Message("环境变量文件不是有效的 UTF-8 文本".into()))?;
    let mut values = BTreeMap::new();
    for (index, original) in content.lines().enumerate() {
        let line = original.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let (key, raw_value) = line.split_once('=').ok_or_else(|| {
            AppError::Message(format!(
                "环境变量文件第 {} 行缺少 =；编辑前请手工整理此行",
                index + 1
            ))
        })?;
        let key = key.trim();
        validate_environment_key(key)?;
        values.insert(
            key.to_string(),
            parse_environment_value(raw_value.trim(), index + 1)?,
        );
    }
    Ok(values)
}

fn parse_environment_value(value: &str, line: usize) -> AppResult<String> {
    if value.starts_with('\'') {
        let mut output = String::new();
        let mut escaped = false;
        let mut closed = false;
        for character in value[1..].chars() {
            if escaped {
                if character == '\'' {
                    output.push('\'');
                } else {
                    output.push('\\');
                    output.push(character);
                }
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '\'' {
                closed = true;
                break;
            } else {
                output.push(character);
            }
        }
        if !closed || escaped {
            return Err(AppError::Message(format!(
                "环境变量文件第 {line} 行的单引号未闭合"
            )));
        }
        return Ok(output);
    }
    if value.starts_with('"') {
        let mut output = String::new();
        let mut escaped = false;
        let mut closed = false;
        for character in value[1..].chars() {
            if escaped {
                output.push(match character {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    other => other,
                });
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                closed = true;
                break;
            } else {
                output.push(character);
            }
        }
        if !closed || escaped {
            return Err(AppError::Message(format!(
                "环境变量文件第 {line} 行的双引号未闭合"
            )));
        }
        return Ok(output);
    }
    Ok(value
        .split_once(" #")
        .map(|(value, _)| value)
        .unwrap_or(value)
        .trim_end()
        .to_string())
}

fn serialize_environment(values: &BTreeMap<String, String>) -> Vec<u8> {
    let mut output = String::new();
    for (key, value) in values {
        output.push_str(key);
        output.push_str("='");
        output.push_str(&value.replace('\'', "\\'"));
        output.push_str("'\n");
    }
    output.into_bytes()
}

fn validate_environment_key(key: &str) -> AppResult<()> {
    let mut characters = key.chars();
    if !characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
        || !characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
        || key.len() > 128
    {
        return Err(AppError::Message(format!("环境变量名 {key} 无效")));
    }
    Ok(())
}

fn managed_environment(
    server_id: &str,
    project_name: &str,
) -> AppResult<Option<BTreeMap<String, String>>> {
    Ok(load_managed_environments(server_id)?.remove(project_name))
}

fn load_managed_environments(
    server_id: &str,
) -> AppResult<BTreeMap<String, BTreeMap<String, String>>> {
    let entry = ssh::credential_entry(server_id, "compose-environments")?;
    match entry.get_password() {
        Ok(value) => serde_json::from_str(&value)
            .map_err(|error| AppError::Message(format!("无法读取 Compose 凭据：{error}"))),
        Err(keyring::Error::NoEntry) => Ok(BTreeMap::new()),
        Err(error) => Err(AppError::Keyring(error.to_string())),
    }
}

pub(crate) fn export_managed_environments(
    server_id: &str,
) -> AppResult<HashMap<String, HashMap<String, String>>> {
    Ok(load_managed_environments(server_id)?
        .into_iter()
        .map(|(project, values)| (project, values.into_iter().collect()))
        .collect())
}

pub(crate) fn import_managed_environments(
    server_id: &str,
    environments: &HashMap<String, HashMap<String, String>>,
) -> AppResult<()> {
    let mut all = load_managed_environments(server_id)?;
    for (project, values) in environments {
        validate_project_name(project)?;
        for key in values.keys() {
            validate_environment_key(key)?;
        }
        all.insert(
            project.clone(),
            values
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        );
    }
    store_managed_environments(server_id, &all)
}

fn save_managed_environment(
    server_id: &str,
    project_name: &str,
    environment: Option<BTreeMap<String, String>>,
) -> AppResult<()> {
    let mut all = load_managed_environments(server_id)?;
    match environment {
        Some(environment) => {
            all.insert(project_name.to_string(), environment);
        }
        None => {
            all.remove(project_name);
        }
    }
    store_managed_environments(server_id, &all)
}

fn store_managed_environments(
    server_id: &str,
    all: &BTreeMap<String, BTreeMap<String, String>>,
) -> AppResult<()> {
    let entry = ssh::credential_entry(server_id, "compose-environments")?;
    if all.is_empty() {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(AppError::Keyring(error.to_string())),
        }
    } else {
        let value = serde_json::to_string(&all)
            .map_err(|error| AppError::Message(format!("无法保存 Compose 凭据：{error}")))?;
        entry
            .set_password(&value)
            .map_err(|error| AppError::Keyring(error.to_string()))
    }
}

fn redact(output: &str, mut secrets: Vec<String>) -> String {
    secrets.retain(|value| !value.is_empty());
    secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
    secrets.dedup();
    let mut output = output.to_string();
    for secret in secrets {
        output = output.replace(&secret, "••••••");
    }
    output
}

fn truncate_output(output: &str) -> String {
    if output.len() <= MAX_OUTPUT_BYTES {
        return output.to_string();
    }
    let mut boundary = MAX_OUTPUT_BYTES;
    while !output.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}\n… 输出已截断 …", &output[..boundary])
}

fn acquire_lock(session: &Session, target: &ComposeTarget) -> AppResult<DeploymentLock> {
    let sftp = session.sftp()?;
    let root = target.working_directory.join(".controlvps");
    ensure_directory(&sftp, &root, 0o700)?;
    let path = root.join("deployment.lock");
    sftp.mkdir(&path, 0o700)
        .map_err(|_| AppError::Message("该项目已有部署或回滚正在执行，请完成后再试".into()))?;
    Ok(DeploymentLock { sftp, path })
}

fn ensure_directory(sftp: &Sftp, path: &Path, mode: i32) -> AppResult<()> {
    let mut current = PathBuf::from("/");
    for component in path.components() {
        current.push(component);
        if current == Path::new("/") {
            continue;
        }
        match sftp.mkdir(&current, mode) {
            Ok(()) => {}
            Err(error) => {
                if !sftp.stat(&current)?.is_dir() {
                    return Err(error.into());
                }
            }
        }
    }
    Ok(())
}

fn atomic_write(sftp: &Sftp, path: &Path, content: &[u8], mode: u32) -> AppResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::Message("远程文件路径无效".into()))?;
    let temporary = parent.join(format!(".controlvps-{}.part", Uuid::new_v4()));
    let result: AppResult<()> = (|| {
        let mut file = sftp.open_mode(
            &temporary,
            ssh2::OpenFlags::WRITE | ssh2::OpenFlags::CREATE | ssh2::OpenFlags::TRUNCATE,
            mode as i32,
            ssh2::OpenType::File,
        )?;
        file.write_all(content)?;
        file.flush()?;
        file.setstat(FileStat {
            size: None,
            uid: None,
            gid: None,
            perm: Some(mode),
            atime: None,
            mtime: None,
        })?;
        drop(file);
        if sftp
            .rename(
                &temporary,
                path,
                Some(RenameFlags::ATOMIC | RenameFlags::OVERWRITE | RenameFlags::NATIVE),
            )
            .is_err()
        {
            if sftp
                .rename(&temporary, path, Some(RenameFlags::OVERWRITE))
                .is_err()
            {
                sftp.unlink(path)?;
                sftp.rename(&temporary, path, Some(RenameFlags::NATIVE))?;
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = sftp.unlink(&temporary);
    }
    result
}

fn read_required(sftp: &Sftp, path: &Path, limit: u64) -> AppResult<Vec<u8>> {
    read_optional(sftp, path, limit)?
        .ok_or_else(|| AppError::Message(format!("远程文件不存在：{}", path.to_string_lossy())))
}

fn read_optional(sftp: &Sftp, path: &Path, limit: u64) -> AppResult<Option<Vec<u8>>> {
    let file = match sftp.open(path) {
        Ok(file) => file,
        Err(error) if error.code() == ssh2::ErrorCode::SFTP(2) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut content = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut content)
        .map_err(|error| {
            AppError::Message(format!("读取远程文件 {} 失败：{error}", path.display()))
        })?;
    if content.len() as u64 > limit {
        return Err(AppError::Message(format!(
            "远程文件超过大小限制：{}",
            path.to_string_lossy()
        )));
    }
    Ok(Some(content))
}

fn remove_optional(sftp: &Sftp, path: &Path) -> AppResult<()> {
    match sftp.unlink(path) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == ssh2::ErrorCode::SFTP(2) => Ok(()),
        Err(error) => Err(error.into()),
    }
}
