use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Local;

use crate::commands::AppPaths;
use crate::data_protection;
use crate::db::Database;
use crate::models::{
    NotificationEventKind, NotificationSeverity, RunbookExecutionRequest, RunbookTrigger,
};
use crate::notifications::{self, NotificationDispatchEvent};
use crate::runbook::{self, RunbookExecutionRegistry};

const EXPIRY_DIGEST_KEY: &str = "last-expiry-digest-date";

pub struct RunbookScheduler {
    stopped: Arc<AtomicBool>,
}

impl RunbookScheduler {
    pub fn start(db: Database, paths: &AppPaths, registry: RunbookExecutionRegistry) -> Self {
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = stopped.clone();
        let known_hosts = paths.known_hosts.clone();
        let running_backups: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let _ = db.recover_interrupted_runbook_executions();
        std::thread::spawn(move || {
            let mut last_expiry_check: Option<Instant> = None;
            while !worker_stopped.load(Ordering::Relaxed) {
                // 1. Runbook schedules
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

                // 2. Scheduled Restic backups
                if let Ok(policies) = db.due_data_protection_policies() {
                    for policy in policies {
                        if worker_stopped.load(Ordering::Relaxed) {
                            break;
                        }
                        let acquired = running_backups
                            .lock()
                            .map(|mut set| set.insert(policy.id.clone()))
                            .unwrap_or(false);
                        if !acquired {
                            continue;
                        }
                        // Advance next_run_at immediately so the loop does not re-trigger while running.
                        let _ = db.mark_data_protection_scheduled_started(&policy.id);
                        let backup_db = db.clone();
                        let backup_known_hosts = known_hosts.clone();
                        let backup_policy_id = policy.id.clone();
                        let running_guard = running_backups.clone();
                        let _ = std::thread::Builder::new()
                            .name(format!("restic-sched-{}", &policy.id[..8.min(policy.id.len())]))
                            .spawn(move || {
                                let result = data_protection::backup(
                                    &backup_db,
                                    &backup_policy_id,
                                    &backup_known_hosts,
                                    false,
                                );
                                match result {
                                    Ok(_) => {
                                        let _ = backup_db.mark_data_protection_scheduled_result(
                                            &backup_policy_id,
                                            true,
                                            None,
                                        );
                                    }
                                    Err(error) => {
                                        let _ = backup_db.mark_data_protection_scheduled_result(
                                            &backup_policy_id,
                                            false,
                                            Some(&error.to_string()),
                                        );
                                    }
                                }
                                if let Ok(mut set) = running_guard.lock() {
                                    set.remove(&backup_policy_id);
                                }
                            });
                    }
                }

                // 3. Daily server expiration digest (checked every 5 minutes, dispatched at most once/day)
                if last_expiry_check
                    .is_none_or(|instant| instant.elapsed() >= Duration::from_secs(300))
                {
                    last_expiry_check = Some(Instant::now());
                    check_expiring_servers_digest(&db);
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

fn check_expiring_servers_digest(db: &Database) {
    let today = Local::now().format("%Y-%m-%d").to_string();
    if db
        .get_app_setting(EXPIRY_DIGEST_KEY)
        .ok()
        .flatten()
        .as_deref()
        == Some(today.as_str())
    {
        return;
    }
    let Ok(summary) = db.action_center_summary() else {
        return;
    };
    let urgent: Vec<_> = summary
        .expiring_servers
        .iter()
        .filter(|item| item.days_remaining <= 7)
        .collect();
    if urgent.is_empty() {
        return;
    }
    let _ = db.set_app_setting(EXPIRY_DIGEST_KEY, &today);
    let lines = urgent
        .iter()
        .take(8)
        .map(|item| {
            let status = if item.days_remaining < 0 {
                format!("已过期 {} 天", item.days_remaining.abs())
            } else if item.days_remaining == 0 {
                "今天到期".into()
            } else {
                format!("{} 天后到期", item.days_remaining)
            };
            format!("• {}（{}，到期日 {}）", item.server_name, status, item.expires_at)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let has_expired = urgent.iter().any(|item| item.days_remaining <= 1);
    notifications::dispatch_async(
        db.clone(),
        NotificationDispatchEvent {
            kind: NotificationEventKind::ServerExpiry,
            severity: if has_expired {
                NotificationSeverity::Critical
            } else {
                NotificationSeverity::Warning
            },
            title: format!("{} 台 VPS 即将到期或已过期", urgent.len()),
            message: lines,
            target: Some(format!("共 {} 台", urgent.len())),
        },
    );
}
