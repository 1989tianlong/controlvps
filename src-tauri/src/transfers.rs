use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use chrono::Utc;
use uuid::Uuid;

use crate::db::Database;
use crate::error::{AppError, AppResult};
use crate::files;
use crate::models::{FileTransfer, FileTransferKind, FileTransferStatus, FileTransferTask};
use crate::ssh::{self, ConnectionJob};

const MAX_CONCURRENT_TRANSFERS: usize = 2;

struct TransferRecord {
    task: FileTransferTask,
    job: ConnectionJob,
    db: Database,
    known_hosts: PathBuf,
    allow_unknown_host: bool,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct TransferInner {
    records: HashMap<String, TransferRecord>,
    queue: VecDeque<String>,
    active: usize,
}

#[derive(Clone, Default)]
pub struct TransferRegistry {
    inner: Arc<Mutex<TransferInner>>,
}

impl TransferRegistry {
    pub fn enqueue(
        &self,
        db: Database,
        kind: FileTransferKind,
        job: ConnectionJob,
        local_path: String,
        remote_path: String,
        file_name: String,
        known_hosts: PathBuf,
        allow_unknown_host: bool,
    ) -> AppResult<FileTransferTask> {
        let id = Uuid::new_v4().to_string();
        let total_bytes = if kind == FileTransferKind::Upload {
            std::fs::metadata(&local_path)?.len()
        } else {
            0
        };
        let task = FileTransferTask {
            id: id.clone(),
            server_id: job.server.id.clone(),
            server_name: job.server.name.clone(),
            kind,
            status: FileTransferStatus::Queued,
            file_name,
            local_path,
            remote_path,
            total_bytes,
            transferred_bytes: 0,
            bytes_per_second: 0,
            error: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: None,
            finished_at: None,
        };
        let record = TransferRecord {
            task: task.clone(),
            job,
            db,
            known_hosts,
            allow_unknown_host,
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let mut inner = self.inner.lock().map_err(|_| registry_error())?;
        inner.records.insert(id.clone(), record);
        inner.queue.push_back(id);
        drop(inner);
        self.schedule();
        Ok(task)
    }

    pub fn list(&self) -> AppResult<Vec<FileTransferTask>> {
        let inner = self.inner.lock().map_err(|_| registry_error())?;
        let mut tasks = inner
            .records
            .values()
            .map(|record| record.task.clone())
            .collect::<Vec<_>>();
        tasks.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        Ok(tasks)
    }

    pub fn cancel(&self, id: &str) -> AppResult<FileTransferTask> {
        let mut inner = self.inner.lock().map_err(|_| registry_error())?;
        let record = inner
            .records
            .get_mut(id)
            .ok_or_else(|| AppError::Message("未找到这个传输任务".into()))?;
        match record.task.status {
            FileTransferStatus::Queued => {
                record.cancel.store(true, Ordering::Release);
                record.task.status = FileTransferStatus::Cancelled;
                record.task.finished_at = Some(Utc::now().to_rfc3339());
            }
            FileTransferStatus::Running => record.cancel.store(true, Ordering::Release),
            _ => {}
        }
        Ok(record.task.clone())
    }

    pub fn cancel_by_server(&self, server_id: &str) -> AppResult<()> {
        let ids = self
            .inner
            .lock()
            .map_err(|_| registry_error())?
            .records
            .iter()
            .filter(|(_, record)| record.task.server_id == server_id)
            .filter(|(_, record)| {
                matches!(
                    record.task.status,
                    FileTransferStatus::Queued | FileTransferStatus::Running
                )
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            self.cancel(&id)?;
        }
        Ok(())
    }

    pub fn retry(&self, id: &str) -> AppResult<FileTransferTask> {
        let task = {
            let mut inner = self.inner.lock().map_err(|_| registry_error())?;
            let record = inner
                .records
                .get_mut(id)
                .ok_or_else(|| AppError::Message("未找到这个传输任务".into()))?;
            if matches!(
                record.task.status,
                FileTransferStatus::Queued | FileTransferStatus::Running
            ) {
                return Err(AppError::Message("只能重试已失败或已取消的任务".into()));
            }
            record.cancel.store(false, Ordering::Release);
            record.task.status = FileTransferStatus::Queued;
            record.task.transferred_bytes = 0;
            record.task.bytes_per_second = 0;
            record.task.error = None;
            record.task.started_at = None;
            record.task.finished_at = None;
            record.task.clone()
        };
        self.inner
            .lock()
            .map_err(|_| registry_error())?
            .queue
            .push_back(id.to_string());
        self.schedule();
        Ok(task)
    }

    pub fn clear_finished(&self, server_id: &str) -> AppResult<()> {
        let mut inner = self.inner.lock().map_err(|_| registry_error())?;
        inner.records.retain(|_, record| {
            record.task.server_id != server_id
                || matches!(
                    record.task.status,
                    FileTransferStatus::Queued | FileTransferStatus::Running
                )
        });
        Ok(())
    }

    fn schedule(&self) {
        loop {
            let next = {
                let mut inner = match self.inner.lock() {
                    Ok(inner) => inner,
                    Err(_) => return,
                };
                if inner.active >= MAX_CONCURRENT_TRANSFERS {
                    return;
                }
                let mut next = None;
                while let Some(id) = inner.queue.pop_front() {
                    let Some(record) = inner.records.get_mut(&id) else {
                        continue;
                    };
                    if record.task.status != FileTransferStatus::Queued {
                        continue;
                    }
                    let execution = {
                        let record = inner.records.get_mut(&id).expect("record checked above");
                        record.task.status = FileTransferStatus::Running;
                        record.task.started_at = Some(Utc::now().to_rfc3339());
                        (
                            record.job.clone(),
                            record.known_hosts.clone(),
                            record.allow_unknown_host,
                            record.cancel.clone(),
                        )
                    };
                    inner.active += 1;
                    next = Some((id, execution.0, execution.1, execution.2, execution.3));
                    break;
                }
                next
            };
            let Some((id, job, known_hosts, allow_unknown_host, cancel)) = next else {
                return;
            };
            let registry = self.clone();
            std::thread::spawn(move || {
                registry.run(id, job, known_hosts, allow_unknown_host, cancel);
            });
        }
    }

    fn run(
        &self,
        id: String,
        job: ConnectionJob,
        known_hosts: PathBuf,
        allow_unknown_host: bool,
        cancel: Arc<AtomicBool>,
    ) {
        let Some((kind, db, local_path, remote_path, file_name)) = self.execution(&id) else {
            return;
        };
        let started = Instant::now();
        let registry = self.clone();
        let progress_id = id.clone();
        let result = ssh::connect(
            &job.server,
            &job.credentials,
            job.jump_host.as_ref(),
            &known_hosts,
            allow_unknown_host,
        )
        .and_then(|connection| {
            let progress = |transferred, total| {
                registry.progress(&progress_id, transferred, total, started);
            };
            match kind {
                FileTransferKind::Upload => {
                    let remote_directory = Path::new(&remote_path)
                        .parent()
                        .unwrap_or_else(|| Path::new("/"))
                        .to_string_lossy()
                        .into_owned();
                    files::upload_with_progress(
                        &connection.session,
                        &local_path,
                        &remote_directory,
                        &file_name,
                        &cancel,
                        progress,
                    )
                }
                FileTransferKind::Download => files::download_with_progress(
                    &connection.session,
                    &remote_path,
                    &local_path,
                    &cancel,
                    progress,
                ),
            }
        });
        let action = match kind {
            FileTransferKind::Upload => "文件上传",
            FileTransferKind::Download => "文件下载",
        };
        let (success, summary) = match &result {
            Ok(transfer) => (
                true,
                format!("已传输 {}，{} 字节", transfer.remote_path, transfer.bytes),
            ),
            Err(error) if cancel.load(Ordering::Acquire) => (false, "传输已取消".into()),
            Err(error) => (false, error.to_string()),
        };
        self.finish(&id, result, cancel);
        let _ = db.log_operation(&job.server.id, action, success, &summary);
    }

    fn execution(&self, id: &str) -> Option<(FileTransferKind, Database, String, String, String)> {
        let inner = self.inner.lock().ok()?;
        let record = inner.records.get(id)?;
        Some((
            record.task.kind,
            record.db.clone(),
            record.task.local_path.clone(),
            record.task.remote_path.clone(),
            record.task.file_name.clone(),
        ))
    }

    fn progress(&self, id: &str, transferred: u64, total: u64, started: Instant) {
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(record) = inner.records.get_mut(id) {
                if record.task.status == FileTransferStatus::Running {
                    record.task.transferred_bytes = transferred;
                    record.task.total_bytes = total;
                    let elapsed = started.elapsed().as_secs_f64();
                    record.task.bytes_per_second = if elapsed > 0.0 {
                        (transferred as f64 / elapsed) as u64
                    } else {
                        0
                    };
                }
            }
        }
    }

    fn finish(&self, id: &str, result: AppResult<FileTransfer>, cancel: Arc<AtomicBool>) {
        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(_) => return,
        };
        if let Some(record) = inner.records.get_mut(id) {
            record.task.finished_at = Some(Utc::now().to_rfc3339());
            record.task.bytes_per_second = 0;
            match result {
                Ok(transfer) => {
                    record.task.status = FileTransferStatus::Completed;
                    record.task.transferred_bytes = transfer.bytes;
                    record.task.total_bytes = transfer.bytes;
                }
                Err(error) => {
                    record.task.status = if cancel.load(Ordering::Acquire) {
                        FileTransferStatus::Cancelled
                    } else {
                        FileTransferStatus::Failed
                    };
                    record.task.error = if cancel.load(Ordering::Acquire) {
                        None
                    } else {
                        Some(error.to_string())
                    };
                }
            }
        }
        inner.active = inner.active.saturating_sub(1);
        drop(inner);
        self.schedule();
    }
}

fn registry_error() -> AppError {
    AppError::Message("文件传输队列不可用".into())
}
