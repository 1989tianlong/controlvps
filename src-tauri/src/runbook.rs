use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::batch;
use crate::db::Database;
use crate::error::{AppError, AppResult};
use crate::models::{
    Runbook, RunbookExecution, RunbookExecutionRequest, RunbookExecutionStatus, RunbookResult,
    RunbookServerResult, RunbookStepOutcome, RunbookStepResult,
};
use crate::ssh::{self, ConnectionJob};

#[derive(Default)]
pub struct ExecutionControl {
    cancelled: AtomicBool,
}

impl ExecutionControl {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

#[derive(Clone, Default)]
pub struct RunbookExecutionRegistry {
    controls: Arc<Mutex<HashMap<String, Arc<ExecutionControl>>>>,
}

impl RunbookExecutionRegistry {
    fn insert(&self, id: &str, control: Arc<ExecutionControl>) -> AppResult<()> {
        self.controls
            .lock()
            .map_err(|_| AppError::Message("Runbook 执行注册表不可用".into()))?
            .insert(id.to_string(), control);
        Ok(())
    }

    fn remove(&self, id: &str) {
        if let Ok(mut controls) = self.controls.lock() {
            controls.remove(id);
        }
    }

    pub fn cancel(&self, id: &str) -> AppResult<()> {
        let controls = self
            .controls
            .lock()
            .map_err(|_| AppError::Message("Runbook 执行注册表不可用".into()))?;
        let control = controls.get(id).ok_or_else(|| {
            AppError::Message("任务未在运行，可能已经结束或已被应用重启中断".into())
        })?;
        control.cancel();
        Ok(())
    }

    pub fn contains(&self, id: &str) -> bool {
        self.controls
            .lock()
            .is_ok_and(|controls| controls.contains_key(id))
    }
}

pub fn start_execution(
    db: Database,
    known_hosts: PathBuf,
    registry: RunbookExecutionRegistry,
    mut request: RunbookExecutionRequest,
) -> AppResult<RunbookExecution> {
    validate_request(&request)?;
    let mut unique_ids = Vec::new();
    for id in request.target_ids {
        if !unique_ids.contains(&id) {
            unique_ids.push(id);
        }
    }
    request.target_ids = unique_ids;
    if request.trigger == crate::models::RunbookTrigger::Retry {
        request.canary_count = if request.target_ids.len() > 1 {
            request.canary_count.min(request.target_ids.len() - 1)
        } else {
            0
        };
    }
    let execution = db.create_runbook_execution(&request)?;
    let id = execution.id.clone();
    let control = Arc::new(ExecutionControl::default());
    registry.insert(&id, control.clone())?;
    let worker_db = db.clone();
    let worker_registry = registry.clone();
    let worker_id = id.clone();
    let spawn = std::thread::Builder::new()
        .name(format!("runbook-{id}"))
        .spawn(move || {
            let _ = worker_db.mark_runbook_execution_running(&worker_id);
            if control.cancelled.load(Ordering::Relaxed) {
                let _ = worker_db.finish_runbook_execution(
                    &worker_id,
                    RunbookExecutionStatus::Cancelled,
                    None,
                    Some("任务在连接服务器前已取消"),
                );
                worker_registry.remove(&worker_id);
                return;
            }
            let jobs = match build_jobs(&worker_db, &request.target_ids) {
                Ok(jobs) => jobs,
                Err(error) => {
                    let message = error.to_string();
                    let _ = worker_db.finish_runbook_execution(
                        &worker_id,
                        RunbookExecutionStatus::Failed,
                        None,
                        Some(&message),
                    );
                    worker_registry.remove(&worker_id);
                    return;
                }
            };
            match execute_with_control(
                jobs,
                request.runbook,
                known_hosts,
                request.concurrency,
                request.canary_count,
                Duration::from_secs(request.timeout_seconds),
                control,
            ) {
                Ok(result) => {
                    log_result(&worker_db, &result);
                    let status = execution_status(&result, &worker_id, &worker_registry);
                    let _ =
                        worker_db.finish_runbook_execution(&worker_id, status, Some(&result), None);
                }
                Err(error) => {
                    let message = error.to_string();
                    let _ = worker_db.finish_runbook_execution(
                        &worker_id,
                        RunbookExecutionStatus::Failed,
                        None,
                        Some(&message),
                    );
                }
            }
            worker_registry.remove(&worker_id);
        });
    if let Err(error) = spawn {
        registry.remove(&id);
        let message = format!("无法启动后台执行线程：{error}");
        db.finish_runbook_execution(&id, RunbookExecutionStatus::Failed, None, Some(&message))?;
        return Err(AppError::Message(message));
    }
    Ok(execution)
}

fn execution_status(
    result: &RunbookResult,
    execution_id: &str,
    registry: &RunbookExecutionRegistry,
) -> RunbookExecutionStatus {
    let cancelled = registry
        .controls
        .lock()
        .ok()
        .and_then(|controls| controls.get(execution_id).cloned())
        .is_some_and(|control| control.cancelled.load(Ordering::Relaxed));
    if cancelled
        || result.results.iter().any(|server| {
            server
                .steps
                .iter()
                .any(|step| step.outcome == RunbookStepOutcome::Cancelled)
        })
    {
        RunbookExecutionStatus::Cancelled
    } else if result.results.iter().any(|server| {
        server
            .steps
            .iter()
            .any(|step| step.outcome == RunbookStepOutcome::TimedOut)
    }) {
        RunbookExecutionStatus::TimedOut
    } else if result.failure_count == 0 && result.skipped_count == 0 {
        RunbookExecutionStatus::Succeeded
    } else if result.success_count > 0 {
        RunbookExecutionStatus::Partial
    } else {
        RunbookExecutionStatus::Failed
    }
}

fn build_jobs(db: &Database, ids: &[String]) -> AppResult<Vec<ConnectionJob>> {
    ids.iter().map(|id| connection_job(db, id)).collect()
}

pub fn connection_job(db: &Database, id: &str) -> AppResult<ConnectionJob> {
    let mut server = db.get_server(id)?;
    let mut credentials = ssh::load_credentials(&server)?;
    apply_group_defaults(db, &mut server, &mut credentials)?;
    let mut jumps = Vec::new();
    for jump_id in effective_jump_ids(db, &server)? {
        let jump_server = db.get_server(&jump_id)?;
        jumps.push(ssh::JumpHost {
            credentials: ssh::load_credentials(&jump_server)?,
            server: jump_server,
        });
    }
    Ok(ConnectionJob {
        server,
        credentials,
        jumps,
    })
}

fn apply_group_defaults(
    db: &Database,
    server: &mut crate::models::Server,
    credentials: &mut ssh::Credentials,
) -> AppResult<()> {
    let Some(group_name) = server.group_name.as_deref() else {
        return Ok(());
    };
    let Ok(group) = db.get_server_group_by_name(group_name) else {
        return Ok(());
    };
    if server.inherit_port {
        if let Some(port) = group.default_port {
            server.port = port;
        }
    }
    if server.inherit_username {
        if let Some(username) = group
            .default_username
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            server.username = username.to_string();
        }
    }
    if server.auth_profile_id.is_none() && server.inherit_auth {
        server.auth_profile_id = group.auth_profile_id.clone();
    }
    if let Some(profile_id) = server.auth_profile_id.as_deref() {
        let profile = db.get_auth_profile(profile_id)?;
        server.auth_method = profile.auth_method;
        server.private_key_path = profile.private_key_path;
        credentials.password = ssh::profile_secret(&profile.id, "password")?;
        credentials.key_passphrase = ssh::profile_secret(&profile.id, "key-passphrase")?;
    }
    Ok(())
}

fn effective_jump_ids(db: &Database, server: &crate::models::Server) -> AppResult<Vec<String>> {
    let mut ids = explicit_jump_ids(server);
    if ids.is_empty() && server.inherit_jump {
        if let Some(group_name) = server.group_name.as_deref() {
            if let Ok(group) = db.get_server_group_by_name(group_name) {
                if let Some(jump_id) = group
                    .jump_server_id
                    .filter(|jump_id| jump_id != &server.id)
                {
                    ids = inherited_route(db, &jump_id, 0)?;
                }
            }
        }
    }
    if ids.len() > 4 {
        return Err(AppError::Message("跳板链最多支持 4 跳".into()));
    }
    Ok(ids)
}

fn explicit_jump_ids(server: &crate::models::Server) -> Vec<String> {
    let ids: Vec<String> = if server.jump_chain.is_empty() {
        server.jump_server_id.iter().cloned().collect()
    } else {
        server.jump_chain.clone()
    };
    let mut seen = std::collections::HashSet::new();
    ids.into_iter()
        .filter(|jump_id| !jump_id.trim().is_empty() && jump_id != &server.id)
        .filter(|jump_id| seen.insert(jump_id.clone()))
        .collect()
}

fn inherited_route(db: &Database, jump_id: &str, depth: usize) -> AppResult<Vec<String>> {
    if depth > 4 {
        return Err(AppError::Message("分组跳板的上级跳板层级过深".into()));
    }
    let jump_server = db.get_server(jump_id)?;
    let upstream = explicit_jump_ids(&jump_server);
    let mut ids = if jump_server.inherit_jump && upstream.is_empty() {
        match jump_server.group_name.as_deref() {
            Some(group_name) => match db.get_server_group_by_name(group_name) {
                Ok(group) => match group.jump_server_id {
                    Some(next) => inherited_route(db, &next, depth + 1)?,
                    None => Vec::new(),
                },
                Err(_) => Vec::new(),
            },
            None => Vec::new(),
        }
    } else {
        upstream
    };
    if !ids.iter().any(|existing| existing == jump_id) {
        ids.push(jump_id.to_string());
    }
    Ok(ids)
}

fn validate_request(request: &RunbookExecutionRequest) -> AppResult<()> {
    if request.target_ids.is_empty() || request.target_ids.len() > 50 {
        return Err(AppError::Message(
            "每次 Runbook 执行需要选择 1 到 50 台服务器".into(),
        ));
    }
    if !(1..=10).contains(&request.concurrency) {
        return Err(AppError::Message("并发数必须在 1 到 10 之间".into()));
    }
    if !(10..=86_400).contains(&request.timeout_seconds) {
        return Err(AppError::Message(
            "单步超时时长必须在 10 秒到 24 小时之间".into(),
        ));
    }
    if request.trigger != crate::models::RunbookTrigger::Retry
        && (request.canary_count > 10 || request.canary_count >= request.target_ids.len())
    {
        return Err(AppError::Message(
            "灰度首批必须为 0 到 10 台，且小于目标总数".into(),
        ));
    }
    Ok(())
}

fn log_result(db: &Database, result: &RunbookResult) {
    for server in &result.results {
        if let Some(error) = &server.error {
            let _ = db.log_operation(
                &server.server_id,
                &format!("Runbook · {}", result.runbook_name),
                false,
                error,
            );
        }
        for step in &server.steps {
            let summary = match step.exit_code {
                Some(code) => format!("退出码 {code}，耗时 {} ms", step.duration_ms),
                None => step.error.clone().unwrap_or_else(|| "执行失败".into()),
            };
            let _ = db.log_operation(
                &server.server_id,
                &format!("Runbook · {} · {}", result.runbook_name, step.step_name),
                step.success,
                &summary,
            );
        }
    }
}

pub fn execute(
    jobs: Vec<ConnectionJob>,
    runbook: Runbook,
    known_hosts: PathBuf,
    concurrency: usize,
    canary_count: usize,
) -> AppResult<RunbookResult> {
    execute_with_control(
        jobs,
        runbook,
        known_hosts,
        concurrency,
        canary_count,
        Duration::from_secs(86_400),
        Arc::new(ExecutionControl::default()),
    )
}

fn execute_with_control(
    jobs: Vec<ConnectionJob>,
    runbook: Runbook,
    known_hosts: PathBuf,
    concurrency: usize,
    canary_count: usize,
    timeout: Duration,
    control: Arc<ExecutionControl>,
) -> AppResult<RunbookResult> {
    let started = Instant::now();
    let total = jobs.len();
    let canary_count = canary_count.min(total);
    let mut results = execute_jobs(
        &jobs[..canary_count],
        &runbook,
        &known_hosts,
        concurrency,
        timeout,
        &control,
    )?;
    let canary_failed = results.iter().any(|result| !result.success);
    if !canary_failed && !control.cancelled.load(Ordering::Relaxed) {
        results.extend(execute_jobs(
            &jobs[canary_count..],
            &runbook,
            &known_hosts,
            concurrency,
            timeout,
            &control,
        )?);
    }
    let success_count = results.iter().filter(|result| result.success).count();
    let skipped_count = total - results.len();
    Ok(RunbookResult {
        runbook_name: runbook.name,
        failure_count: results.len() - success_count,
        success_count,
        skipped_count,
        results,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

fn execute_jobs(
    jobs: &[ConnectionJob],
    runbook: &Runbook,
    known_hosts: &PathBuf,
    concurrency: usize,
    timeout: Duration,
    control: &Arc<ExecutionControl>,
) -> AppResult<Vec<RunbookServerResult>> {
    if jobs.is_empty() {
        return Ok(Vec::new());
    }
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::with_capacity(jobs.len()));
    let workers = concurrency.clamp(1, jobs.len());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                if control.cancelled.load(Ordering::Relaxed) {
                    break;
                }
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(job) = jobs.get(index) else {
                    break;
                };
                let result = execute_job(job, runbook, known_hosts, timeout, control);
                if let Ok(mut results) = results.lock() {
                    results.push((index, result));
                }
            });
        }
    });
    let mut results = results
        .into_inner()
        .map_err(|_| AppError::Message("Runbook 执行结果锁不可用".into()))?;
    results.sort_by_key(|(index, _)| *index);
    Ok(results.into_iter().map(|(_, result)| result).collect())
}

fn execute_job(
    job: &ConnectionJob,
    runbook: &Runbook,
    known_hosts: &PathBuf,
    timeout: Duration,
    control: &Arc<ExecutionControl>,
) -> RunbookServerResult {
    let started = Instant::now();
    let connection = match ssh::connect(
        &job.server,
        &job.credentials,
        &job.jumps,
        known_hosts,
        false,
    ) {
        Ok(connection) => connection,
        Err(error) => {
            return RunbookServerResult {
                server_id: job.server.id.clone(),
                server_name: job.server.name.clone(),
                steps: Vec::new(),
                success: false,
                error: Some(error.to_string()),
                duration_ms: started.elapsed().as_millis() as u64,
            };
        }
    };
    let mut steps = Vec::with_capacity(runbook.steps.len());
    let mut success = true;
    for step in &runbook.steps {
        if control.cancelled.load(Ordering::Relaxed) {
            success = false;
            break;
        }
        let step_started = Instant::now();
        let command = batch::render_command(&step.command, &job.server);
        match ssh::execute_controlled(&connection.session, &command, timeout, &control.cancelled) {
            Ok(ssh::ControlledExecOutput::Completed(output)) => {
                let step_success = output.exit_code == 0;
                let mut text = output.stdout;
                if !output.stderr.trim().is_empty() {
                    text.push_str(&output.stderr);
                }
                steps.push(RunbookStepResult {
                    step_id: step.id.clone(),
                    step_name: step.name.clone(),
                    command,
                    output: text,
                    exit_code: Some(output.exit_code),
                    success: step_success,
                    outcome: if step_success {
                        RunbookStepOutcome::Succeeded
                    } else {
                        RunbookStepOutcome::Failed
                    },
                    error: None,
                    duration_ms: step_started.elapsed().as_millis() as u64,
                });
                if !step_success {
                    success = false;
                    break;
                }
            }
            Ok(ssh::ControlledExecOutput::Cancelled { stdout, stderr }) => {
                steps.push(controlled_failure(
                    step,
                    command,
                    stdout,
                    stderr,
                    RunbookStepOutcome::Cancelled,
                    "任务已取消；SSH 通道已关闭，远端进程状态需要人工核对",
                    step_started.elapsed(),
                ));
                success = false;
                break;
            }
            Ok(ssh::ControlledExecOutput::TimedOut { stdout, stderr }) => {
                steps.push(controlled_failure(
                    step,
                    command,
                    stdout,
                    stderr,
                    RunbookStepOutcome::TimedOut,
                    &format!(
                        "步骤超过 {} 秒；SSH 通道已关闭，远端进程状态需要人工核对",
                        timeout.as_secs()
                    ),
                    step_started.elapsed(),
                ));
                success = false;
                break;
            }
            Err(error) => {
                steps.push(RunbookStepResult {
                    step_id: step.id.clone(),
                    step_name: step.name.clone(),
                    command,
                    output: String::new(),
                    exit_code: None,
                    success: false,
                    outcome: RunbookStepOutcome::Failed,
                    error: Some(error.to_string()),
                    duration_ms: step_started.elapsed().as_millis() as u64,
                });
                success = false;
                break;
            }
        }
    }
    RunbookServerResult {
        server_id: job.server.id.clone(),
        server_name: job.server.name.clone(),
        steps,
        success,
        error: None,
        duration_ms: started.elapsed().as_millis() as u64,
    }
}

fn controlled_failure(
    step: &crate::models::RunbookStep,
    command: String,
    stdout: String,
    stderr: String,
    outcome: RunbookStepOutcome,
    error: &str,
    duration: Duration,
) -> RunbookStepResult {
    let mut output = stdout;
    if !stderr.trim().is_empty() {
        output.push_str(&stderr);
    }
    RunbookStepResult {
        step_id: step.id.clone(),
        step_name: step.name.clone(),
        command,
        output,
        exit_code: None,
        success: false,
        outcome,
        error: Some(error.to_string()),
        duration_ms: duration.as_millis() as u64,
    }
}
