use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use crate::batch;
use crate::error::{AppError, AppResult};
use crate::models::{Runbook, RunbookResult, RunbookServerResult, RunbookStepResult};
use crate::ssh::{self, ConnectionJob};

pub fn execute(
    jobs: Vec<ConnectionJob>,
    runbook: Runbook,
    known_hosts: PathBuf,
    concurrency: usize,
    canary_count: usize,
) -> AppResult<RunbookResult> {
    let started = Instant::now();
    let total = jobs.len();
    let canary_count = canary_count.min(total);
    let mut results = execute_jobs(&jobs[..canary_count], &runbook, &known_hosts, concurrency)?;
    let canary_failed = results.iter().any(|result| !result.success);
    if !canary_failed {
        results.extend(execute_jobs(
            &jobs[canary_count..],
            &runbook,
            &known_hosts,
            concurrency,
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
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(job) = jobs.get(index) else {
                    break;
                };
                let result = execute_job(job, runbook, known_hosts);
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
) -> RunbookServerResult {
    let started = Instant::now();
    let connection = match ssh::connect(
        &job.server,
        &job.credentials,
        job.jump_host.as_ref(),
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
        let step_started = Instant::now();
        let command = batch::render_command(&step.command, &job.server);
        match ssh::execute(&connection.session, &command) {
            Ok(output) => {
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
                    error: None,
                    duration_ms: step_started.elapsed().as_millis() as u64,
                });
                if !step_success {
                    success = false;
                    break;
                }
            }
            Err(error) => {
                steps.push(RunbookStepResult {
                    step_id: step.id.clone(),
                    step_name: step.name.clone(),
                    command,
                    output: String::new(),
                    exit_code: None,
                    success: false,
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
