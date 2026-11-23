use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use crate::error::{AppError, AppResult};
use crate::models::{BatchExecutionResult, BatchResult, Server};
use crate::ssh::{self, Credentials};

pub struct BatchJob {
    pub server: Server,
    pub credentials: Credentials,
}

pub fn execute(
    jobs: Vec<BatchJob>,
    template: String,
    known_hosts: PathBuf,
    concurrency: usize,
) -> AppResult<BatchResult> {
    let started = Instant::now();
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::with_capacity(jobs.len()));
    let workers = concurrency.clamp(1, jobs.len().max(1));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(job) = jobs.get(index) else {
                    break;
                };
                let result = execute_job(job, &template, &known_hosts);
                if let Ok(mut results) = results.lock() {
                    results.push((index, result));
                }
            });
        }
    });
    let mut results = results
        .into_inner()
        .map_err(|_| AppError::Message("批量执行结果锁不可用".into()))?;
    results.sort_by_key(|(index, _)| *index);
    let results = results
        .into_iter()
        .map(|(_, result)| result)
        .collect::<Vec<_>>();
    let success_count = results.iter().filter(|result| result.success).count();
    Ok(BatchResult {
        failure_count: results.len() - success_count,
        success_count,
        results,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

fn execute_job(job: &BatchJob, template: &str, known_hosts: &PathBuf) -> BatchExecutionResult {
    let started = Instant::now();
    let command = render_command(template, &job.server);
    let result =
        ssh::connect(&job.server, &job.credentials, known_hosts, false).and_then(|connection| {
            let output = ssh::execute(&connection.session, &command)?;
            let mut text = output.stdout;
            if !output.stderr.trim().is_empty() {
                text.push_str(&output.stderr);
            }
            Ok((text, output.exit_code))
        });
    let (output, exit_code, success, error) = match result {
        Ok((output, exit_code)) => (output, Some(exit_code), exit_code == 0, None),
        Err(error) => (String::new(), None, false, Some(error.to_string())),
    };
    BatchExecutionResult {
        server_id: job.server.id.clone(),
        server_name: job.server.name.clone(),
        command,
        output,
        exit_code,
        success,
        error,
        duration_ms: started.elapsed().as_millis() as u64,
    }
}

pub fn render_command(template: &str, server: &Server) -> String {
    template
        .replace("{{hostname}}", &shell_quote(&server.host))
        .replace("{{username}}", &shell_quote(&server.username))
        .replace("{{name}}", &shell_quote(&server.name))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
