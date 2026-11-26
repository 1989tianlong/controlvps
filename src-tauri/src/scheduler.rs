use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::commands::AppPaths;
use crate::db::Database;
use crate::models::{RunbookExecutionRequest, RunbookTrigger};
use crate::runbook::{self, RunbookExecutionRegistry};

pub struct RunbookScheduler {
    stopped: Arc<AtomicBool>,
}

impl RunbookScheduler {
    pub fn start(db: Database, paths: &AppPaths, registry: RunbookExecutionRegistry) -> Self {
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = stopped.clone();
        let known_hosts = paths.known_hosts.clone();
        let _ = db.recover_interrupted_runbook_executions();
        std::thread::spawn(move || {
            while !worker_stopped.load(Ordering::Relaxed) {
                if let Ok(schedules) = db.due_runbook_schedules() {
                    for schedule in schedules {
                        if worker_stopped.load(Ordering::Relaxed) {
                            break;
                        }
                        if schedule
                            .last_execution_id
                            .as_deref()
                            .is_some_and(|id| registry.contains(id))
                        {
                            let _ = db.mark_runbook_schedule_error(
                                &schedule.id,
                                "上一次执行仍未结束，本次已跳过，避免重复变更",
                            );
                            continue;
                        }
                        let request = RunbookExecutionRequest {
                            runbook: match db.get_runbook(&schedule.runbook_id) {
                                Ok(runbook) => runbook,
                                Err(error) => {
                                    let _ = db.mark_runbook_schedule_error(
                                        &schedule.id,
                                        &error.to_string(),
                                    );
                                    continue;
                                }
                            },
                            target_ids: schedule.server_ids.clone(),
                            concurrency: schedule.concurrency,
                            canary_count: schedule.canary_count,
                            timeout_seconds: schedule.timeout_seconds,
                            trigger: RunbookTrigger::Scheduled,
                            schedule_id: Some(schedule.id.clone()),
                            retry_of: None,
                        };
                        match runbook::start_execution(
                            db.clone(),
                            known_hosts.clone(),
                            registry.clone(),
                            request,
                        ) {
                            Ok(execution) => {
                                let _ =
                                    db.mark_runbook_schedule_started(&schedule.id, &execution.id);
                            }
                            Err(error) => {
                                let _ = db
                                    .mark_runbook_schedule_error(&schedule.id, &error.to_string());
                            }
                        }
                    }
                }
                for _ in 0..10 {
                    if worker_stopped.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        });
        Self { stopped }
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }
}

impl Drop for RunbookScheduler {
    fn drop(&mut self) {
        self.stop();
    }
}
