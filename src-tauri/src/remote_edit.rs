use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use crate::db::Database;
use crate::error::{AppError, AppResult};
use crate::files;
use crate::ssh::{self, ConnectionJob};

const MAX_EDIT_FILE_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEditSession {
    pub id: String,
    pub server_id: String,
    pub server_name: String,
    pub remote_path: String,
    pub local_path: String,
    pub editor: String,
    pub status: String,
    pub last_synced_at: Option<String>,
    pub sync_count: u32,
    pub last_error: Option<String>,
}

struct ActiveSession {
    info: RemoteEditSession,
    cancel: Arc<AtomicBool>,
    temp_dir: PathBuf,
}

#[derive(Default, Clone)]
pub struct RemoteEditRegistry {
    sessions: Arc<Mutex<HashMap<String, ActiveSession>>>,
}

impl RemoteEditRegistry {
    pub fn list(&self, server_id: Option<&str>) -> Vec<RemoteEditSession> {
        let Ok(guard) = self.sessions.lock() else {
            return Vec::new();
        };
        guard
            .values()
            .filter(|s| server_id.is_none_or(|id| s.info.server_id == id))
            .map(|s| s.info.clone())
            .collect()
    }

    pub fn stop(&self, session_id: &str) -> bool {
        let removed = self
            .sessions
            .lock()
            .ok()
            .and_then(|mut guard| guard.remove(session_id));
        if let Some(session) = removed {
            session.cancel.store(true, Ordering::Relaxed);
            let _ = fs::remove_dir_all(&session.temp_dir);
            return true;
        }
        false
    }

    pub fn stop_all(&self) {
        if let Ok(mut guard) = self.sessions.lock() {
            for (_, session) in guard.drain() {
                session.cancel.store(true, Ordering::Relaxed);
                let _ = fs::remove_dir_all(&session.temp_dir);
            }
        }
    }

    fn update_status(
        &self,
        app: &AppHandle,
        session_id: &str,
        status: &str,
        increment_sync: bool,
        last_error: Option<String>,
    ) {
        let snapshot = {
            let Ok(mut guard) = self.sessions.lock() else {
                return;
            };
            let Some(active) = guard.get_mut(session_id) else {
                return;
            };
            active.info.status = status.to_string();
            if increment_sync {
                active.info.sync_count += 1;
                active.info.last_synced_at = Some(Utc::now().to_rfc3339());
            }
            active.info.last_error = last_error;
            active.info.clone()
        };
        let _ = app.emit("remote-edit-updated", snapshot);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn open_in_local_editor(
    app: AppHandle,
    registry: RemoteEditRegistry,
    db: Database,
    job: ConnectionJob,
    known_hosts: PathBuf,
    allow_unknown_host: bool,
    data_dir: &Path,
    remote_path: &str,
    editor: &str,
) -> AppResult<RemoteEditSession> {
    let remote = Path::new(remote_path.trim());
    let file_name = remote
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty() && *n != "." && *n != "..")
        .ok_or_else(|| AppError::Message("无效的远程文件路径".into()))?
        .to_string();
    let remote_dir = remote
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "/".to_string());

    let connection = ssh::connect(
        &job.server,
        &job.credentials,
        &job.jumps,
        &known_hosts,
        allow_unknown_host,
    )?;
    let sftp = connection.session.sftp()?;
    let stat = sftp.stat(remote)?;
    if stat.is_dir() {
        connection.disconnect();
        return Err(AppError::Message("不能使用本地编辑器直接打开目录".into()));
    }
    let size = stat.size.unwrap_or(0);
    if size > MAX_EDIT_FILE_BYTES {
        connection.disconnect();
        return Err(AppError::Message(format!(
            "文件大小超过 5 MiB 限制（当前 {:.1} MiB），请使用下载功能",
            size as f64 / (1024.0 * 1024.0)
        )));
    }

    let session_id = Uuid::new_v4().to_string();
    let temp_dir = data_dir.join("remote-edit").join(&session_id);
    fs::create_dir_all(&temp_dir)?;
    let local_path = temp_dir.join(&file_name);
    let local_path_str = local_path.to_string_lossy().into_owned();

    files::download(&connection.session, remote_path, &local_path_str)?;
    connection.disconnect();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&local_path, fs::Permissions::from_mode(0o600));
    }

    let initial_hash = file_sha256(&local_path)?;
    launch_editor(editor, &local_path)?;

    let info = RemoteEditSession {
        id: session_id.clone(),
        server_id: job.server.id.clone(),
        server_name: job.server.name.clone(),
        remote_path: remote_path.to_string(),
        local_path: local_path_str.clone(),
        editor: editor.to_string(),
        status: "watching".into(),
        last_synced_at: None,
        sync_count: 0,
        last_error: None,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut guard) = registry.sessions.lock() {
        guard.insert(
            session_id.clone(),
            ActiveSession {
                info: info.clone(),
                cancel: cancel.clone(),
                temp_dir: temp_dir.clone(),
            },
        );
    }

    let registry_bg = registry.clone();
    let app_bg = app.clone();
    let session_id_bg = session_id.clone();
    std::thread::spawn(move || {
        let mut last_hash = initial_hash;
        while !cancel.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(700));
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            if !local_path.is_file() {
                continue;
            }
            let Ok(current_hash) = file_sha256(&local_path) else {
                continue;
            };
            if current_hash == last_hash {
                continue;
            }
            // Brief debounce so atomic editor writes finish before upload.
            std::thread::sleep(Duration::from_millis(250));
            let Ok(stable_hash) = file_sha256(&local_path) else {
                continue;
            };
            if stable_hash != current_hash {
                continue;
            }
            registry_bg.update_status(&app_bg, &session_id_bg, "syncing", false, None);
            let upload_result = ssh::connect(
                &job.server,
                &job.credentials,
                &job.jumps,
                &known_hosts,
                allow_unknown_host,
            )
            .and_then(|conn| {
                let res = files::upload(&conn.session, &local_path_str, &remote_dir, &file_name);
                conn.disconnect();
                res
            });
            match upload_result {
                Ok(_) => {
                    last_hash = stable_hash;
                    let _ = db.log_operation(
                        &job.server.id,
                        "本地编辑同步",
                        true,
                        &format!("本地编辑器保存并同步到 {}", remote_dir_join(&remote_dir, &file_name)),
                    );
                    registry_bg.update_status(&app_bg, &session_id_bg, "synced", true, None);
                }
                Err(err) => {
                    registry_bg.update_status(
                        &app_bg,
                        &session_id_bg,
                        "error",
                        false,
                        Some(err.to_string()),
                    );
                }
            }
        }
    });

    Ok(info)
}

fn remote_dir_join(dir: &str, name: &str) -> String {
    if dir == "/" {
        format!("/{name}")
    } else {
        format!("{}/{name}", dir.trim_end_matches('/'))
    }
}

fn file_sha256(path: &Path) -> AppResult<String> {
    let bytes = fs::read(path)?;
    let digest = Sha256::digest(&bytes);
    Ok(format!("{digest:x}"))
}

fn launch_editor(editor: &str, local_path: &Path) -> AppResult<()> {
    let path_str = local_path.to_string_lossy().into_owned();
    let result = match editor {
        "vscode" => try_spawn("code", &[&path_str])
            .or_else(|_| try_spawn("open", &["-a", "Visual Studio Code", &path_str])),
        "cursor" => try_spawn("cursor", &[&path_str])
            .or_else(|_| try_spawn("open", &["-a", "Cursor", &path_str])),
        "zed" => try_spawn("zed", &[&path_str])
            .or_else(|_| try_spawn("open", &["-a", "Zed", &path_str])),
        _ => {
            #[cfg(target_os = "macos")]
            {
                try_spawn("open", &["-t", &path_str])
            }
            #[cfg(not(target_os = "macos"))]
            {
                try_spawn("xdg-open", &[&path_str])
            }
        }
    };
    result.map_err(|err| AppError::Message(format!("无法启动本地编辑器 ({editor})：{err}")))
}

fn try_spawn(bin: &str, args: &[&str]) -> Result<(), String> {
    Command::new(bin)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|err| err.to_string())
}
