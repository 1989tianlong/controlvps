use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::{DateTime, Utc};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::db::Database;
use crate::error::{AppError, AppResult};
use crate::models::{
    BackupPreview, BackupRestoreResult, BackupSummary, PortForwardInput, RunbookInput,
    RunbookStepInput, Server, ServerInput, ServiceMonitorInput, SnippetInput, WorkspaceBackup,
};
use crate::ssh;
use crate::utils;

const MAGIC: &str = "CONTROLVPS";
const FORMAT_VERSION: u32 = 1;
const MAX_BACKUP_BYTES: u64 = 64 * 1024 * 1024;
const AEAD_CONTEXT: &[u8] = b"controlvps-workspace-backup-v1";
const ARGON2_M_COST: u32 = 19_456;
const ARGON2_T_COST: u32 = 2;
const ARGON2_P_COST: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KdfParams {
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EncryptedEnvelope {
    magic: String,
    format_version: u32,
    kdf: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kdf_params: Option<KdfParams>,
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
    let mut plaintext = serde_json::to_vec(&backup)
        .map_err(|error| AppError::Message(format!("无法序列化备份：{error}")))?;
    let envelope = encrypt(&plaintext, password);
    utils::zeroize_bytes(&mut plaintext);
    let envelope = envelope?;
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
    let kdf_params = KdfParams {
        m_cost: ARGON2_M_COST,
        t_cost: ARGON2_T_COST,
        p_cost: ARGON2_P_COST,
    };
    let cipher = cipher(password, &salt, Some(kdf_params))?;
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
        kdf_params: Some(kdf_params),
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
    decrypt_envelope(&envelope, password)
}

fn decrypt_envelope(envelope: &EncryptedEnvelope, password: &str) -> AppResult<WorkspaceBackup> {
    if envelope.magic != MAGIC
        || envelope.format_version != FORMAT_VERSION
        || envelope.kdf != "argon2id"
        || envelope.cipher != "aes-256-gcm"
    {
        return Err(AppError::Message("备份格式或版本不受支持".into()));
    }
    let salt = BASE64.decode(&envelope.salt).map_err(|_| invalid_backup())?;
    let nonce = BASE64
        .decode(&envelope.nonce)
        .map_err(|_| invalid_backup())?;
    let ciphertext = BASE64
        .decode(&envelope.ciphertext)
        .map_err(|_| invalid_backup())?;
    if salt.len() != 16 || nonce.len() != 12 {
        return Err(invalid_backup());
    }
    let cipher = cipher(password, &salt, envelope.kdf_params)?;
    let mut plaintext = cipher
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad: AEAD_CONTEXT,
            },
        )
        .map_err(|_| AppError::Message("备份密码错误或文件已损坏".into()))?;
    let parsed = serde_json::from_slice(&plaintext).map_err(|_| invalid_backup());
    utils::zeroize_bytes(&mut plaintext);
    parsed
}

fn cipher(password: &str, salt: &[u8], kdf_params: Option<KdfParams>) -> AppResult<Aes256Gcm> {
    let mut key = [0_u8; 32];
    let argon2 = match kdf_params {
        Some(spec) => {
            if !(8_192..=524_288).contains(&spec.m_cost)
                || !(1..=20).contains(&spec.t_cost)
                || !(1..=16).contains(&spec.p_cost)
            {
                return Err(invalid_backup());
            }
            let params = Params::new(spec.m_cost, spec.t_cost, spec.p_cost, Some(32))
                .map_err(|_| invalid_backup())?;
            Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        }
        None => Argon2::default(),
    };
    let hash_result = argon2.hash_password_into(password.as_bytes(), salt, &mut key);
    if hash_result.is_err() {
        utils::zeroize_bytes(&mut key);
        return Err(AppError::Message("无法派生备份加密密钥".into()));
    }
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|_| AppError::Message("无法初始化备份加密".into()));
    utils::zeroize_bytes(&mut key);
    cipher
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
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    write_result
}

const BYO_SYNC_SETTING_KEY: &str = "byo-sync";
const BYO_SYNC_FILENAME: &str = "controlvps-vault.cvpsbackup";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ByoSyncConfig {
    pub folder_path: String,
    pub include_credentials: bool,
    pub last_synced_at: Option<String>,
    pub last_sync_hash: Option<String>,
    pub last_direction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ByoSyncStatus {
    pub config: ByoSyncConfig,
    pub vault_file_path: Option<String>,
    pub remote_exists: bool,
    pub remote_updated_at: Option<String>,
    pub remote_hash: Option<String>,
    pub remote_size_bytes: Option<u64>,
    pub remote_has_changes: bool,
}

pub fn load_byo_sync_config(db: &Database) -> AppResult<ByoSyncConfig> {
    let Some(raw) = db.get_app_setting(BYO_SYNC_SETTING_KEY)? else {
        return Ok(ByoSyncConfig::default());
    };
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

pub fn save_byo_sync_config(db: &Database, config: &ByoSyncConfig) -> AppResult<()> {
    let encoded = serde_json::to_string(config)
        .map_err(|err| AppError::Message(format!("无法序列化同步配置：{err}")))?;
    db.set_app_setting(BYO_SYNC_SETTING_KEY, &encoded)
}

pub fn update_byo_sync_config(
    db: &Database,
    folder_path: String,
    include_credentials: bool,
) -> AppResult<ByoSyncStatus> {
    let mut config = load_byo_sync_config(db)?;
    let trimmed = folder_path.trim().to_string();
    if !trimmed.is_empty() && !Path::new(&trimmed).is_dir() {
        return Err(AppError::Message("所选同步目录不存在或不是文件夹".into()));
    }
    config.folder_path = trimmed;
    config.include_credentials = include_credentials;
    save_byo_sync_config(db, &config)?;
    byo_sync_status(db)
}

pub fn byo_sync_status(db: &Database) -> AppResult<ByoSyncStatus> {
    let config = load_byo_sync_config(db)?;
    if config.folder_path.trim().is_empty() {
        return Ok(ByoSyncStatus {
            config,
            vault_file_path: None,
            remote_exists: false,
            remote_updated_at: None,
            remote_hash: None,
            remote_size_bytes: None,
            remote_has_changes: false,
        });
    }
    let vault_path = PathBuf::from(&config.folder_path).join(BYO_SYNC_FILENAME);
    if !vault_path.is_file() {
        return Ok(ByoSyncStatus {
            config,
            vault_file_path: Some(vault_path.display().to_string()),
            remote_exists: false,
            remote_updated_at: None,
            remote_hash: None,
            remote_size_bytes: None,
            remote_has_changes: false,
        });
    }
    let meta = std::fs::metadata(&vault_path)?;
    let bytes = std::fs::read(&vault_path)?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let modified = meta
        .modified()
        .ok()
        .map(|ts| DateTime::<Utc>::from(ts).to_rfc3339());
    let remote_has_changes = config.last_sync_hash.as_deref() != Some(hash.as_str());
    Ok(ByoSyncStatus {
        config,
        vault_file_path: Some(vault_path.display().to_string()),
        remote_exists: true,
        remote_updated_at: modified,
        remote_hash: Some(hash),
        remote_size_bytes: Some(meta.len()),
        remote_has_changes,
    })
}

pub fn byo_sync_push(db: &Database, password: &str) -> AppResult<ByoSyncStatus> {
    let mut config = load_byo_sync_config(db)?;
    if config.folder_path.trim().is_empty() {
        return Err(AppError::Message("请先选择同步目录（如 iCloud Drive / Dropbox / Syncthing 目录）".into()));
    }
    let folder = PathBuf::from(&config.folder_path);
    if !folder.is_dir() {
        return Err(AppError::Message("同步目录不存在，请重新选择".into()));
    }
    let vault_path = folder.join(BYO_SYNC_FILENAME);
    create(db, &vault_path, password, config.include_credentials)?;
    let bytes = std::fs::read(&vault_path)?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    config.last_synced_at = Some(Utc::now().to_rfc3339());
    config.last_sync_hash = Some(hash);
    config.last_direction = Some("push".into());
    save_byo_sync_config(db, &config)?;
    byo_sync_status(db)
}

pub fn byo_sync_pull(db: &Database, password: &str) -> AppResult<BackupRestoreResult> {
    let mut config = load_byo_sync_config(db)?;
    if config.folder_path.trim().is_empty() {
        return Err(AppError::Message("请先选择同步目录".into()));
    }
    let vault_path = PathBuf::from(&config.folder_path).join(BYO_SYNC_FILENAME);
    if !vault_path.is_file() {
        return Err(AppError::Message(
            "同步目录中尚未找到 controlvps-vault.cvpsbackup 文件".into(),
        ));
    }
    let result = restore(db, &vault_path, password)?;
    let bytes = std::fs::read(&vault_path)?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    config.last_synced_at = Some(Utc::now().to_rfc3339());
    config.last_sync_hash = Some(hash);
    config.last_direction = Some("pull".into());
    save_byo_sync_config(db, &config)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MonitoringSettings;

    fn sample_backup() -> WorkspaceBackup {
        WorkspaceBackup {
            format_version: 1,
            created_at: "2026-11-27T00:00:00Z".into(),
            servers: Vec::new(),
            snippets: Vec::new(),
            runbooks: Vec::new(),
            port_forwards: Vec::new(),
            service_monitors: Vec::new(),
            monitoring: MonitoringSettings::default(),
            service_webhook_enabled: Some(false),
            service_webhook_url: None,
            includes_credentials: false,
            credentials: HashMap::new(),
            compose_environments: HashMap::new(),
        }
    }

    #[test]
    fn encrypt_decrypt_roundtrip_and_wrong_password_rejection() {
        let backup = sample_backup();
        let bytes = serde_json::to_vec(&backup).expect("serialize");
        let envelope = encrypt(&bytes, "strong-passphrase-123").expect("encrypt");
        assert!(envelope.kdf_params.is_some());

        let restored = decrypt_envelope(&envelope, "strong-passphrase-123").expect("decrypt");
        assert_eq!(restored.created_at, backup.created_at);

        let wrong = decrypt_envelope(&envelope, "wrong-passphrase-999");
        assert!(wrong.is_err());

        // Legacy v1 envelopes without explicit kdf_params should also decrypt cleanly.
        let mut legacy = envelope;
        legacy.kdf_params = None;
        let legacy_ok = decrypt_envelope(&legacy, "strong-passphrase-123");
        assert!(legacy_ok.is_ok());
    }
}
