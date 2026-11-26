use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::Argon2;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::Database;
use crate::error::{AppError, AppResult};
use crate::models::{
    BackupPreview, BackupRestoreResult, BackupSummary, PortForwardInput, RunbookInput,
    RunbookStepInput, Server, ServerInput, ServiceMonitorInput, SnippetInput, WorkspaceBackup,
};
use crate::ssh;

const MAGIC: &str = "CONTROLVPS";
const FORMAT_VERSION: u32 = 1;
const MAX_BACKUP_BYTES: u64 = 64 * 1024 * 1024;
const AEAD_CONTEXT: &[u8] = b"controlvps-workspace-backup-v1";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EncryptedEnvelope {
    magic: String,
    format_version: u32,
    kdf: String,
    cipher: String,
    salt: String,
    nonce: String,
    ciphertext: String,
}

pub fn create(
    db: &Database,
    path: &Path,
    password: &str,
    include_credentials: bool,
) -> AppResult<BackupSummary> {
    validate_password(password)?;
    let mut backup = db.workspace_backup()?;
    backup.includes_credentials = include_credentials;
    if include_credentials {
        backup.service_webhook_url = crate::service_monitor::webhook_url()?;
        for server in &backup.servers {
            let compose_environments = crate::compose::export_managed_environments(&server.id)?;
            if !compose_environments.is_empty() {
                backup
                    .compose_environments
                    .insert(server.id.clone(), compose_environments);
            }
            let credentials = ssh::load_credentials(server)?;
            if credentials.password.is_some() || credentials.key_passphrase.is_some() {
                backup.credentials.insert(
                    server.id.clone(),
                    crate::models::BackupCredentials {
                        password: credentials.password,
                        key_passphrase: credentials.key_passphrase,
                    },
                );
            }
        }
    }
    let plaintext = serde_json::to_vec(&backup)
        .map_err(|error| AppError::Message(format!("无法序列化备份：{error}")))?;
    let envelope = encrypt(&plaintext, password)?;
    let encoded = serde_json::to_vec_pretty(&envelope)
        .map_err(|error| AppError::Message(format!("无法生成备份文件：{error}")))?;
    atomic_write(path, &encoded)?;
    Ok(BackupSummary {
        path: path.display().to_string(),
        created_at: backup.created_at,
        size_bytes: encoded.len() as u64,
        server_count: backup.servers.len(),
        snippet_count: backup.snippets.len(),
        runbook_count: backup.runbooks.len(),
        port_forward_count: backup.port_forwards.len(),
        service_monitor_count: backup.service_monitors.len(),
        includes_credentials: include_credentials,
    })
}

pub fn preview(db: &Database, path: &Path, password: &str) -> AppResult<BackupPreview> {
    let backup = decrypt_file(path, password)?;
    validate_backup(&backup)?;
    let existing = db.list_servers()?;
    let existing_server_count = backup
        .servers
        .iter()
        .filter(|server| {
            existing.iter().any(|item| {
                item.host == server.host
                    && item.port == server.port
                    && item.username == server.username
            })
        })
        .count();
    Ok(BackupPreview {
        created_at: backup.created_at,
        server_count: backup.servers.len(),
        new_server_count: backup.servers.len() - existing_server_count,
        existing_server_count,
        snippet_count: backup.snippets.len(),
        runbook_count: backup.runbooks.len(),
        port_forward_count: backup.port_forwards.len(),
        service_monitor_count: backup.service_monitors.len(),
        includes_credentials: backup.includes_credentials,
    })
}

pub fn restore(db: &Database, path: &Path, password: &str) -> AppResult<BackupRestoreResult> {
    let backup = decrypt_file(path, password)?;
    validate_backup(&backup)?;
    let mut result = BackupRestoreResult {
        imported_servers: 0,
        skipped_servers: 0,
        imported_snippets: 0,
        skipped_snippets: 0,
        imported_runbooks: 0,
        skipped_runbooks: 0,
        imported_port_forwards: 0,
        skipped_port_forwards: 0,
        imported_service_monitors: 0,
        skipped_service_monitors: 0,
    };
    let existing_servers = db.list_servers()?;
    let server_ids = backup
        .servers
        .iter()
        .map(|server| server.id.clone())
        .collect::<Vec<String>>();
    let mut context = RestoreContext {
        db,
        backup: &backup,
        existing_servers,
        server_map: HashMap::new(),
        importing: HashSet::new(),
        imported: 0,
        skipped: 0,
    };
    for id in server_ids {
        context.import_server(&id).map_err(|error| {
            AppError::Message(format!(
                "恢复中断：{error}。已合并的项目会保留，重新恢复时会跳过重复项"
            ))
        })?;
    }
    result.imported_servers = context.imported;
    result.skipped_servers = context.skipped;

    let existing_snippets = db.list_snippets()?;
    for snippet in &backup.snippets {
        if existing_snippets
            .iter()
            .any(|item| item.name.eq_ignore_ascii_case(&snippet.name))
        {
            result.skipped_snippets += 1;
            continue;
        }
        db.create_snippet(&SnippetInput {
            name: snippet.name.clone(),
            description: snippet.description.clone(),
            command: snippet.command.clone(),
        })?;
        result.imported_snippets += 1;
    }
    let existing_runbooks = db.list_runbooks()?;
    for runbook in &backup.runbooks {
        if existing_runbooks
            .iter()
            .any(|item| item.name.eq_ignore_ascii_case(&runbook.name))
        {
            result.skipped_runbooks += 1;
            continue;
        }
        db.create_runbook(&RunbookInput {
            name: runbook.name.clone(),
            description: runbook.description.clone(),
            steps: runbook
                .steps
                .iter()
                .map(|step| RunbookStepInput {
                    name: step.name.clone(),
                    command: step.command.clone(),
                })
                .collect(),
        })?;
        result.imported_runbooks += 1;
    }
    let existing_forwards = db.list_port_forwards()?;
    for rule in &backup.port_forwards {
        let Some(server_id) = context.server_map.get(&rule.server_id) else {
            result.skipped_port_forwards += 1;
            continue;
        };
        if existing_forwards.iter().any(|item| {
            item.server_id == *server_id
                && item.kind == rule.kind
                && item.listen_address == rule.listen_address
                && item.listen_port == rule.listen_port
                && item.destination_host == rule.destination_host
                && item.destination_port == rule.destination_port
        }) {
            result.skipped_port_forwards += 1;
            continue;
        }
        db.create_port_forward(&PortForwardInput {
            name: rule.name.clone(),
            server_id: server_id.clone(),
            kind: rule.kind,
            listen_address: rule.listen_address.clone(),
            listen_port: rule.listen_port,
            destination_host: rule.destination_host.clone(),
            destination_port: rule.destination_port,
        })?;
        result.imported_port_forwards += 1;
    }
    let existing_monitors = db.list_service_monitors()?;
    for monitor in &backup.service_monitors {
        if existing_monitors.iter().any(|item| {
            item.monitor.name.eq_ignore_ascii_case(&monitor.name)
                && item.monitor.kind == monitor.kind
                && item.monitor.target == monitor.target
        }) {
            result.skipped_service_monitors += 1;
            continue;
        }
        let server_id = monitor
            .server_id
            .as_ref()
            .and_then(|old_id| context.server_map.get(old_id))
            .cloned();
        db.create_service_monitor(&ServiceMonitorInput {
            name: monitor.name.clone(),
            kind: monitor.kind,
            target: monitor.target.clone(),
            port: monitor.port,
            server_id,
            interval_seconds: monitor.interval_seconds,
            timeout_seconds: monitor.timeout_seconds,
            expected_status: monitor.expected_status,
            keyword: monitor.keyword.clone(),
            failure_threshold: monitor.failure_threshold,
            certificate_warning_days: monitor.certificate_warning_days,
            enabled: monitor.enabled,
        })?;
        result.imported_service_monitors += 1;
    }
    db.update_monitoring_settings(&backup.monitoring)?;
    if let Some(enabled) = backup.service_webhook_enabled {
        crate::service_monitor::restore_webhook(
            db,
            enabled,
            backup.service_webhook_url.as_deref(),
        )?;
    }
    Ok(result)
}

struct RestoreContext<'a> {
    db: &'a Database,
    backup: &'a WorkspaceBackup,
    existing_servers: Vec<Server>,
    server_map: HashMap<String, String>,
    importing: HashSet<String>,
    imported: usize,
    skipped: usize,
}

impl RestoreContext<'_> {
    fn import_server(&mut self, old_id: &str) -> AppResult<String> {
        if let Some(id) = self.server_map.get(old_id) {
            return Ok(id.clone());
        }
        let server = self
            .backup
            .servers
            .iter()
            .find(|server| server.id == old_id)
            .ok_or_else(|| AppError::Message("备份中的跳板机引用无效".into()))?
            .clone();
        if let Some(existing) = self.existing_servers.iter().find(|item| {
            item.host == server.host && item.port == server.port && item.username == server.username
        }) {
            self.skipped += 1;
            self.server_map
                .insert(old_id.to_string(), existing.id.clone());
            return Ok(existing.id.clone());
        }
        if !self.importing.insert(old_id.to_string()) {
            return Err(AppError::Message("备份中的跳板机引用存在循环".into()));
        }
        let mut jump_chain = Vec::new();
        for hop in &server.jump_chain {
            jump_chain.push(self.import_server(hop)?);
        }
        let jump_server_id = match jump_chain.first() {
            Some(first) => Some(first.clone()),
            None => match server.jump_server_id.as_deref() {
                Some(jump_id) => Some(self.import_server(jump_id)?),
                None => None,
            },
        };
        let credentials = self.backup.credentials.get(old_id);
        let input = ServerInput {
            name: server.name.clone(),
            host: server.host.clone(),
            port: server.port,
            username: server.username.clone(),
            auth_method: server.auth_method.clone(),
            private_key_path: server.private_key_path.clone(),
            password: credentials.and_then(|item| item.password.clone()),
            key_passphrase: credentials.and_then(|item| item.key_passphrase.clone()),
            group_name: server.group_name.clone(),
            tags: server.tags.clone(),
            provider: server.provider.clone(),
            region: server.region.clone(),
            notes: server.notes.clone(),
            cost: server.cost,
            currency: server.currency.clone(),
            expires_at: server.expires_at.clone(),
            jump_server_id,
            jump_chain,
            auth_profile_id: None,
            cloud_account_id: None,
            cloud_resource_id: None,
            inherit_port: server.inherit_port,
            inherit_username: server.inherit_username,
            inherit_jump: server.inherit_jump,
            inherit_auth: server.inherit_auth,
        };
        let imported = self.db.create_server(&input)?;
        if let Err(error) = ssh::save_credentials(&imported, &input) {
            let _ = self.db.delete_server(&imported.id);
            return Err(error);
        }
        if let Some(environments) = self.backup.compose_environments.get(old_id) {
            crate::compose::import_managed_environments(&imported.id, environments)?;
        }
        self.importing.remove(old_id);
        self.imported += 1;
        self.server_map
            .insert(old_id.to_string(), imported.id.clone());
        Ok(imported.id)
    }
}

fn encrypt(plaintext: &[u8], password: &str) -> AppResult<EncryptedEnvelope> {
    let mut salt = [0_u8; 16];
    let mut nonce = [0_u8; 12];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce);
    let cipher = cipher(password, &salt)?;
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: AEAD_CONTEXT,
            },
        )
        .map_err(|_| AppError::Message("备份加密失败".into()))?;
    Ok(EncryptedEnvelope {
        magic: MAGIC.into(),
        format_version: FORMAT_VERSION,
        kdf: "argon2id".into(),
        cipher: "aes-256-gcm".into(),
        salt: BASE64.encode(salt),
        nonce: BASE64.encode(nonce),
        ciphertext: BASE64.encode(ciphertext),
    })
}

fn decrypt_file(path: &Path, password: &str) -> AppResult<WorkspaceBackup> {
    validate_password(password)?;
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > MAX_BACKUP_BYTES {
        return Err(AppError::Message("备份文件超过 64 MiB 安全上限".into()));
    }
    let bytes = std::fs::read(path)?;
    let envelope: EncryptedEnvelope = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::Message("不是有效的 ControlVPS 加密备份".into()))?;
    if envelope.magic != MAGIC
        || envelope.format_version != FORMAT_VERSION
        || envelope.kdf != "argon2id"
        || envelope.cipher != "aes-256-gcm"
    {
        return Err(AppError::Message("备份格式或版本不受支持".into()));
    }
    let salt = BASE64.decode(envelope.salt).map_err(|_| invalid_backup())?;
    let nonce = BASE64
        .decode(envelope.nonce)
        .map_err(|_| invalid_backup())?;
    let ciphertext = BASE64
        .decode(envelope.ciphertext)
        .map_err(|_| invalid_backup())?;
    if salt.len() != 16 || nonce.len() != 12 {
        return Err(invalid_backup());
    }
    let cipher = cipher(password, &salt)?;
    let plaintext = cipher
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad: AEAD_CONTEXT,
            },
        )
        .map_err(|_| AppError::Message("备份密码错误或文件已损坏".into()))?;
    serde_json::from_slice(&plaintext).map_err(|_| invalid_backup())
}

fn cipher(password: &str, salt: &[u8]) -> AppResult<Aes256Gcm> {
    let mut key = [0_u8; 32];
    Argon2::default()
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|_| AppError::Message("无法派生备份加密密钥".into()))?;
    Aes256Gcm::new_from_slice(&key).map_err(|_| AppError::Message("无法初始化备份加密".into()))
}

fn validate_password(password: &str) -> AppResult<()> {
    if password.len() < 8 || password.len() > 1024 {
        return Err(AppError::Message("备份密码需要 8 到 1024 字节".into()));
    }
    Ok(())
}

fn validate_backup(backup: &WorkspaceBackup) -> AppResult<()> {
    if backup.format_version != FORMAT_VERSION || backup.servers.len() > 10_000 {
        return Err(AppError::Message("备份内容或版本不受支持".into()));
    }
    Ok(())
}

fn invalid_backup() -> AppError {
    AppError::Message("备份内容无效或已损坏".into())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AppError::Message("备份文件路径无效".into()))?;
    let temporary = parent.join(format!(".{name}.{}.tmp", Uuid::new_v4()));
    let write_result = (|| -> AppResult<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    write_result
}
