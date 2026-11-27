use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{Duration, Utc};
use rand_core::{OsRng, RngCore};
use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::{
    ActionCenterSummary, AiSettings, AlertEvent, AlertKind, AlertStatus, AuthProfile,
    AuthProfileInput, CloudAccount, CloudAccountInput, CloudProvider, DataProtectionAction,
    DataProtectionInput, DataProtectionPolicy, DataProtectionRun, DataProtectionRunStatus,
    ExpiringCertificateItem, ExpiringServerItem, FailedBackupItem, FailedRunbookItem,
    McpCallRecord, McpSettings, McpSettingsInput, McpWritePolicy, MetricSample, MonitoringSettings,
    NotificationChannel, NotificationChannelInput, NotificationChannelKind, NotificationDelivery,
    NotificationEventKind, NotificationSeverity, OperationLog, PortForwardInput, PortForwardRule,
    ProtectionSourceKind, ResticRepositoryKind, Runbook, RunbookExecution,
    RunbookExecutionRequest, RunbookExecutionStatus, RunbookInput, RunbookResult, RunbookSchedule,
    RunbookScheduleInput, RunbookStep, RunbookTrigger, Server, ServerBenchmark, ServerGroup,
    ServerGroupInput, ServerInput, ServerMetrics, ServiceEvent, ServiceEventKind, ServiceMonitor,
    ServiceMonitorInput, ServiceMonitorKind, ServiceMonitorView, ServiceProbe, ServiceSample,
    Snippet, SnippetInput, WorkspaceBackup,
};
#[cfg(not(test))]
use crate::utils;

const SCHEMA_VERSION: i64 = 4;
#[cfg(not(test))]
const MCP_TOKEN_ACCOUNT: &str = "mcp:bearer-token";
const PRUNE_EVERY_N_WRITES: u64 = 50;

#[derive(Clone)]
pub struct Database {
    connection: Arc<Mutex<Connection>>,
    write_counter: Arc<AtomicU64>,
}

impl Database {
    pub fn new(path: &std::path::Path) -> AppResult<Self> {
        let connection = Connection::open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Self::init(connection)
    }

    #[cfg(test)]
    pub fn in_memory() -> AppResult<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut connection: Connection) -> AppResult<Self> {
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS servers (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                host TEXT NOT NULL,
                port INTEGER NOT NULL,
                username TEXT NOT NULL,
                auth_method TEXT NOT NULL,
                private_key_path TEXT,
                group_name TEXT,
                tags TEXT NOT NULL DEFAULT '[]',
                provider TEXT,
                region TEXT,
                notes TEXT,
                cost REAL,
                currency TEXT,
                expires_at TEXT,
                jump_server_id TEXT,
                jump_chain TEXT,
                auth_profile_id TEXT,
                cloud_account_id TEXT,
                cloud_resource_id TEXT,
                inherit_port INTEGER NOT NULL DEFAULT 0,
                inherit_username INTEGER NOT NULL DEFAULT 0,
                inherit_jump INTEGER NOT NULL DEFAULT 0,
                inherit_auth INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                has_password INTEGER NOT NULL DEFAULT 0,
                has_key_passphrase INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS cloud_accounts (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                provider TEXT NOT NULL,
                has_token INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                last_synced_at TEXT
             );
             CREATE TABLE IF NOT EXISTS auth_profiles (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                auth_method TEXT NOT NULL,
                private_key_path TEXT,
                has_password INTEGER NOT NULL DEFAULT 0,
                has_key_passphrase INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS server_groups (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE COLLATE NOCASE,
                jump_server_id TEXT,
                auth_profile_id TEXT,
                default_port INTEGER,
                default_username TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS operation_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                server_id TEXT NOT NULL,
                action TEXT NOT NULL,
                success INTEGER NOT NULL,
                summary TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY(server_id) REFERENCES servers(id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_operation_logs_server
             ON operation_logs(server_id, id DESC);
             CREATE TABLE IF NOT EXISTS mcp_calls (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                tool TEXT NOT NULL,
                target TEXT NOT NULL DEFAULT '',
                success INTEGER NOT NULL,
                duration_ms INTEGER NOT NULL,
                summary TEXT NOT NULL,
                created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS snippets (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT,
                command TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS runbooks (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT,
                steps TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS port_forwards (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                server_id TEXT NOT NULL,
                kind TEXT NOT NULL DEFAULT 'local',
                listen_address TEXT NOT NULL,
                listen_port INTEGER NOT NULL,
                destination_host TEXT NOT NULL,
                destination_port INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(server_id) REFERENCES servers(id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS metric_samples (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                server_id TEXT NOT NULL,
                cpu_usage REAL NOT NULL,
                memory_usage REAL NOT NULL,
                disk_usage REAL NOT NULL,
                network_rx_bytes_per_second REAL NOT NULL,
                network_tx_bytes_per_second REAL NOT NULL,
                load_average REAL NOT NULL,
                collected_at TEXT NOT NULL,
                FOREIGN KEY(server_id) REFERENCES servers(id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_metric_samples_server_time
             ON metric_samples(server_id, collected_at);
             CREATE TABLE IF NOT EXISTS alert_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                server_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                status TEXT NOT NULL,
                message TEXT NOT NULL,
                value REAL,
                threshold REAL,
                created_at TEXT NOT NULL,
                FOREIGN KEY(server_id) REFERENCES servers(id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_alert_events_server
             ON alert_events(server_id, id DESC);
             CREATE TABLE IF NOT EXISTS service_monitors (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                target TEXT NOT NULL,
                port INTEGER,
                server_id TEXT,
                interval_seconds INTEGER NOT NULL,
                timeout_seconds INTEGER NOT NULL,
                expected_status INTEGER NOT NULL DEFAULT 200,
                keyword TEXT,
                failure_threshold INTEGER NOT NULL DEFAULT 3,
                certificate_warning_days INTEGER NOT NULL DEFAULT 30,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(server_id) REFERENCES servers(id) ON DELETE SET NULL
             );
             CREATE TABLE IF NOT EXISTS service_samples (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                monitor_id TEXT NOT NULL,
                success INTEGER NOT NULL,
                latency_ms INTEGER NOT NULL,
                status_code INTEGER,
                detail TEXT NOT NULL,
                certificate_expires_at TEXT,
                certificate_days_remaining INTEGER,
                checked_at TEXT NOT NULL,
                FOREIGN KEY(monitor_id) REFERENCES service_monitors(id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_service_samples_monitor_time
             ON service_samples(monitor_id, id DESC);
             CREATE TABLE IF NOT EXISTS service_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                monitor_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                status TEXT NOT NULL,
                message TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY(monitor_id) REFERENCES service_monitors(id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_service_events_monitor
             ON service_events(monitor_id, id DESC);
             CREATE TABLE IF NOT EXISTS runbook_schedules (
                id TEXT PRIMARY KEY,
                runbook_id TEXT NOT NULL,
                server_ids TEXT NOT NULL,
                interval_minutes INTEGER NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 0,
                concurrency INTEGER NOT NULL DEFAULT 2,
                canary_count INTEGER NOT NULL DEFAULT 0,
                timeout_seconds INTEGER NOT NULL DEFAULT 300,
                next_run_at TEXT,
                last_run_at TEXT,
                last_execution_id TEXT,
                last_error TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(runbook_id) REFERENCES runbooks(id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_runbook_schedules_due
             ON runbook_schedules(enabled, next_run_at);
             CREATE TABLE IF NOT EXISTS runbook_executions (
                id TEXT PRIMARY KEY,
                runbook_id TEXT NOT NULL,
                runbook_name TEXT NOT NULL,
                runbook_snapshot TEXT NOT NULL,
                schedule_id TEXT,
                retry_of TEXT,
                trigger_kind TEXT NOT NULL,
                status TEXT NOT NULL,
                target_ids TEXT NOT NULL,
                concurrency INTEGER NOT NULL,
                canary_count INTEGER NOT NULL,
                timeout_seconds INTEGER NOT NULL,
                success_count INTEGER NOT NULL DEFAULT 0,
                failure_count INTEGER NOT NULL DEFAULT 0,
                skipped_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                started_at TEXT,
                finished_at TEXT,
                result TEXT,
                error TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_runbook_executions_created
             ON runbook_executions(created_at DESC);
             CREATE TABLE IF NOT EXISTS data_protection_policies (
                id TEXT PRIMARY KEY,
                server_id TEXT NOT NULL,
                name TEXT NOT NULL,
                repository_kind TEXT NOT NULL,
                repository TEXT NOT NULL,
                source_kind TEXT NOT NULL,
                paths TEXT NOT NULL DEFAULT '[]',
                volume_name TEXT,
                database_container TEXT,
                database_name TEXT,
                database_user TEXT,
                keep_last INTEGER NOT NULL DEFAULT 7,
                has_repository_password INTEGER NOT NULL DEFAULT 0,
                has_s3_credentials INTEGER NOT NULL DEFAULT 0,
                has_database_password INTEGER NOT NULL DEFAULT 0,
                schedule_enabled INTEGER NOT NULL DEFAULT 0,
                schedule_interval_minutes INTEGER NOT NULL DEFAULT 1440,
                next_run_at TEXT,
                last_run_at TEXT,
                last_status TEXT,
                last_error TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(server_id) REFERENCES servers(id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_data_protection_server
             ON data_protection_policies(server_id, name);
             CREATE TABLE IF NOT EXISTS data_protection_runs (
                id TEXT PRIMARY KEY,
                policy_id TEXT NOT NULL,
                action TEXT NOT NULL,
                status TEXT NOT NULL,
                snapshot_id TEXT,
                target TEXT,
                output TEXT NOT NULL DEFAULT '',
                error TEXT,
                started_at TEXT NOT NULL,
                finished_at TEXT,
                duration_ms INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY(policy_id) REFERENCES data_protection_policies(id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_data_protection_runs_policy
             ON data_protection_runs(policy_id, started_at DESC);
             CREATE TABLE IF NOT EXISTS notification_channels (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                target TEXT NOT NULL DEFAULT '',
                events TEXT NOT NULL DEFAULT '[]',
                min_severity TEXT NOT NULL DEFAULT 'warning',
                quiet_start TEXT,
                quiet_end TEXT,
                has_secret INTEGER NOT NULL DEFAULT 0,
                has_signing_secret INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS notification_deliveries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                channel_id TEXT NOT NULL,
                channel_name TEXT NOT NULL,
                event_kind TEXT NOT NULL,
                severity TEXT NOT NULL,
                title TEXT NOT NULL,
                message TEXT NOT NULL,
                success INTEGER NOT NULL,
                error TEXT,
                attempts INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_notification_deliveries_created
             ON notification_deliveries(id DESC);
             CREATE TABLE IF NOT EXISTS server_benchmarks (
                id TEXT PRIMARY KEY,
                server_id TEXT NOT NULL,
                payload TEXT NOT NULL,
                score INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                FOREIGN KEY(server_id) REFERENCES servers(id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_server_benchmarks_server
             ON server_benchmarks(server_id, created_at DESC);
             CREATE TABLE IF NOT EXISTS app_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
             );",
        )?;
        let current_version: i64 =
            connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if current_version < SCHEMA_VERSION {
            let tx = connection.transaction()?;
            let has_forward_kind = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('port_forwards') WHERE name = 'kind')",
                [],
                |row| row.get::<_, bool>(0),
            )?;
            if !has_forward_kind {
                tx.execute(
                    "ALTER TABLE port_forwards ADD COLUMN kind TEXT NOT NULL DEFAULT 'local'",
                    [],
                )?;
            }
            let has_jump_server = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('servers') WHERE name = 'jump_server_id')",
                [],
                |row| row.get::<_, bool>(0),
            )?;
            if !has_jump_server {
                tx.execute("ALTER TABLE servers ADD COLUMN jump_server_id TEXT", [])?;
            }
            let has_jump_chain = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('servers') WHERE name = 'jump_chain')",
                [],
                |row| row.get::<_, bool>(0),
            )?;
            if !has_jump_chain {
                tx.execute("ALTER TABLE servers ADD COLUMN jump_chain TEXT", [])?;
            }
            for column in ["auth_profile_id", "cloud_account_id", "cloud_resource_id"] {
                let exists = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM pragma_table_info('servers') WHERE name = ?1)",
                    [column],
                    |row| row.get::<_, bool>(0),
                )?;
                if !exists {
                    tx.execute(&format!("ALTER TABLE servers ADD COLUMN {column} TEXT"), [])?;
                }
            }
            for column in [
                "inherit_port",
                "inherit_username",
                "inherit_jump",
                "inherit_auth",
            ] {
                let exists = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM pragma_table_info('servers') WHERE name = ?1)",
                    [column],
                    |row| row.get::<_, bool>(0),
                )?;
                if !exists {
                    tx.execute(
                        &format!(
                            "ALTER TABLE servers ADD COLUMN {column} INTEGER NOT NULL DEFAULT 0"
                        ),
                        [],
                    )?;
                }
            }
            for (column, definition) in [
                ("schedule_enabled", "INTEGER NOT NULL DEFAULT 0"),
                ("schedule_interval_minutes", "INTEGER NOT NULL DEFAULT 1440"),
                ("next_run_at", "TEXT"),
                ("last_run_at", "TEXT"),
                ("last_status", "TEXT"),
                ("last_error", "TEXT"),
            ] {
                let exists = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM pragma_table_info('data_protection_policies') WHERE name = ?1)",
                    [column],
                    |row| row.get::<_, bool>(0),
                )?;
                if !exists {
                    tx.execute(
                        &format!(
                            "ALTER TABLE data_protection_policies ADD COLUMN {column} {definition}"
                        ),
                        [],
                    )?;
                }
            }
            tx.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_metric_samples_collected
                 ON metric_samples(collected_at);
                 CREATE INDEX IF NOT EXISTS idx_service_samples_monitor_checked
                 ON service_samples(monitor_id, checked_at);
                 CREATE INDEX IF NOT EXISTS idx_service_samples_checked
                 ON service_samples(checked_at);
                 CREATE INDEX IF NOT EXISTS idx_data_protection_due
                 ON data_protection_policies(schedule_enabled, next_run_at);",
            )?;
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            tx.commit()?;
        }
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            write_counter: Arc::new(AtomicU64::new(0)),
        })
    }

    fn should_prune(&self) -> bool {
        self.write_counter.fetch_add(1, Ordering::Relaxed) % PRUNE_EVERY_N_WRITES == 0
    }

    fn connection(&self) -> AppResult<std::sync::MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| AppError::Message("数据库锁不可用".into()))
    }

    pub fn list_servers(&self) -> AppResult<Vec<Server>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, name, host, port, username, auth_method, private_key_path,
                    group_name, tags, provider, region, notes, cost, currency,
                    expires_at, jump_server_id, auth_profile_id, cloud_account_id, cloud_resource_id,
                    inherit_port, inherit_username, inherit_jump, inherit_auth,
                    created_at, updated_at, has_password, has_key_passphrase, jump_chain
             FROM servers ORDER BY COALESCE(group_name, ''), name COLLATE NOCASE",
        )?;
        let rows = statement.query_map([], row_to_server)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_server(&self, id: &str) -> AppResult<Server> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT id, name, host, port, username, auth_method, private_key_path,
                        group_name, tags, provider, region, notes, cost, currency,
                        expires_at, jump_server_id, auth_profile_id, cloud_account_id, cloud_resource_id,
                        inherit_port, inherit_username, inherit_jump, inherit_auth,
                        created_at, updated_at, has_password, has_key_passphrase, jump_chain
                 FROM servers WHERE id = ?1",
                [id],
                row_to_server,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => AppError::Message("未找到这台 VPS".into()),
                other => other.into(),
            })
    }

    pub fn create_server(&self, input: &ServerInput) -> AppResult<Server> {
        validate_server(input)?;
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let connection = self.connection()?;
        validate_jump_route(&connection, &id, input.jump_server_id.as_deref(), &input.jump_chain)?;
        connection.execute(
            "INSERT INTO servers (
                id, name, host, port, username, auth_method, private_key_path,
                group_name, tags, provider, region, notes, cost, currency, expires_at,
                jump_server_id, auth_profile_id, cloud_account_id, cloud_resource_id,
                inherit_port, inherit_username, inherit_jump, inherit_auth,
                created_at, updated_at, has_password, has_key_passphrase, jump_chain
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28)",
            params![
                id,
                input.name.trim(),
                input.host.trim(),
                input.port,
                input.username.trim(),
                auth_name(&input.auth_method),
                clean(input.private_key_path.as_deref()),
                clean(input.group_name.as_deref()),
                serde_json::to_string(&input.tags).map_err(|e| AppError::Message(e.to_string()))?,
                clean(input.provider.as_deref()),
                clean(input.region.as_deref()),
                clean(input.notes.as_deref()),
                input.cost,
                clean(input.currency.as_deref()),
                clean(input.expires_at.as_deref()),
                clean(input.jump_server_id.as_deref()),
                clean(input.auth_profile_id.as_deref()),
                clean(input.cloud_account_id.as_deref()),
                clean(input.cloud_resource_id.as_deref()),
                input.inherit_port,
                input.inherit_username,
                input.inherit_jump,
                input.inherit_auth,
                now,
                now,
                input.password.as_deref().is_some_and(|value| !value.is_empty()),
                input.key_passphrase.as_deref().is_some_and(|value| !value.is_empty()),
                serde_json::to_string(&input.jump_chain).map_err(|e| AppError::Message(e.to_string()))?,
            ],
        )?;
        drop(connection);
        self.get_server(&id)
    }

    pub fn update_server(&self, id: &str, input: &ServerInput) -> AppResult<Server> {
        validate_server(input)?;
        let existing = self.get_server(id)?;
        let has_password = match input.password.as_deref() {
            Some(value) => !value.is_empty(),
            None => existing.has_password,
        };
        let has_key_passphrase = match input.key_passphrase.as_deref() {
            Some(value) => !value.is_empty(),
            None => existing.has_key_passphrase,
        };
        let connection = self.connection()?;
        validate_jump_route(&connection, id, input.jump_server_id.as_deref(), &input.jump_chain)?;
        connection.execute(
            "UPDATE servers SET name = ?2, host = ?3, port = ?4, username = ?5,
                auth_method = ?6, private_key_path = ?7, group_name = ?8, tags = ?9,
                provider = ?10, region = ?11, notes = ?12, cost = ?13, currency = ?14,
                expires_at = ?15, jump_server_id = ?16, auth_profile_id = ?17,
                cloud_account_id = ?18, cloud_resource_id = ?19,
                inherit_port = ?20, inherit_username = ?21, inherit_jump = ?22, inherit_auth = ?23,
                updated_at = ?24, has_password = ?25, has_key_passphrase = ?26, jump_chain = ?27
             WHERE id = ?1",
            params![
                id,
                input.name.trim(),
                input.host.trim(),
                input.port,
                input.username.trim(),
                auth_name(&input.auth_method),
                clean(input.private_key_path.as_deref()),
                clean(input.group_name.as_deref()),
                serde_json::to_string(&input.tags).map_err(|e| AppError::Message(e.to_string()))?,
                clean(input.provider.as_deref()),
                clean(input.region.as_deref()),
                clean(input.notes.as_deref()),
                input.cost,
                clean(input.currency.as_deref()),
                clean(input.expires_at.as_deref()),
                clean(input.jump_server_id.as_deref()),
                clean(input.auth_profile_id.as_deref()),
                clean(input.cloud_account_id.as_deref()),
                clean(input.cloud_resource_id.as_deref()),
                input.inherit_port,
                input.inherit_username,
                input.inherit_jump,
                input.inherit_auth,
                Utc::now().to_rfc3339(),
                has_password,
                has_key_passphrase,
                serde_json::to_string(&input.jump_chain).map_err(|e| AppError::Message(e.to_string()))?,
            ],
        )?;
        drop(connection);
        self.get_server(id)
    }

    pub fn ensure_server_deletable(&self, id: &str) -> AppResult<()> {
        let connection = self.connection()?;
        let mut dependents: i64 = connection.query_row(
            "SELECT COUNT(*) FROM servers WHERE jump_server_id = ?1",
            [id],
            |row| row.get(0),
        )?;
        let mut statement = connection.prepare(
            "SELECT jump_chain FROM servers WHERE id <> ?1 AND jump_chain IS NOT NULL",
        )?;
        let rows = statement.query_map([id], |row| row.get::<_, Option<String>>(0))?;
        for row in rows {
            let chain: Vec<String> = row
                .ok()
                .flatten()
                .and_then(|raw| serde_json::from_str(&raw).ok())
                .unwrap_or_default();
            if chain.iter().any(|hop| hop == id) {
                dependents += 1;
            }
        }
        drop(statement);
        if dependents > 0 {
            return Err(AppError::Message(format!(
                "这台 VPS 正被 {dependents} 台服务器用作跳板机，请先修改它们的连接路由"
            )));
        }
        let groups: i64 = connection.query_row(
            "SELECT COUNT(*) FROM server_groups WHERE jump_server_id = ?1",
            [id],
            |row| row.get(0),
        )?;
        if groups > 0 {
            return Err(AppError::Message(format!(
                "这台 VPS 是 {groups} 个分组的默认跳板机，请先修改分组"
            )));
        }
        Ok(())
    }

    pub fn delete_server(&self, id: &str) -> AppResult<()> {
        self.ensure_server_deletable(id)?;
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        // Remove deleted server ID from any runbook schedules so scheduled runs do not break.
        let schedules: Vec<(String, String)> = {
            let mut stmt = tx.prepare("SELECT id, server_ids FROM runbook_schedules")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let now = Utc::now().to_rfc3339();
        for (schedule_id, raw_ids) in schedules {
            let Ok(mut server_ids) = serde_json::from_str::<Vec<String>>(&raw_ids) else {
                continue;
            };
            let before = server_ids.len();
            server_ids.retain(|item| item != id);
            if server_ids.len() != before {
                let encoded = serde_json::to_string(&server_ids)
                    .map_err(|error| AppError::Message(error.to_string()))?;
                if server_ids.is_empty() {
                    tx.execute(
                        "UPDATE runbook_schedules SET server_ids = ?2, enabled = 0, next_run_at = NULL, updated_at = ?3 WHERE id = ?1",
                        params![schedule_id, encoded, now],
                    )?;
                } else {
                    tx.execute(
                        "UPDATE runbook_schedules SET server_ids = ?2, updated_at = ?3 WHERE id = ?1",
                        params![schedule_id, encoded, now],
                    )?;
                }
            }
        }
        tx.execute("DELETE FROM servers WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn list_cloud_accounts(&self) -> AppResult<Vec<CloudAccount>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, name, provider, has_token, created_at, updated_at, last_synced_at
             FROM cloud_accounts ORDER BY name COLLATE NOCASE",
        )?;
        let rows = statement.query_map([], row_to_cloud_account)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_cloud_account(&self, id: &str) -> AppResult<CloudAccount> {
        self.connection()?
            .query_row(
                "SELECT id, name, provider, has_token, created_at, updated_at, last_synced_at
                 FROM cloud_accounts WHERE id = ?1",
                [id],
                row_to_cloud_account,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    AppError::Message("未找到这个云账户".into())
                }
                other => other.into(),
            })
    }

    pub fn create_cloud_account(&self, input: &CloudAccountInput) -> AppResult<CloudAccount> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        self.connection()?.execute(
            "INSERT INTO cloud_accounts (
                id, name, provider, has_token, created_at, updated_at, last_synced_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            params![
                id,
                input.name.trim(),
                cloud_provider_name(input.provider),
                input
                    .token
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty()),
                now,
                now,
            ],
        )?;
        self.get_cloud_account(&id)
    }

    pub fn update_cloud_account(
        &self,
        id: &str,
        input: &CloudAccountInput,
    ) -> AppResult<CloudAccount> {
        let existing = self.get_cloud_account(id)?;
        let has_token = if input
            .token
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            true
        } else {
            existing.has_token
        };
        self.connection()?.execute(
            "UPDATE cloud_accounts SET name = ?2, provider = ?3, has_token = ?4, updated_at = ?5 WHERE id = ?1",
            params![id, input.name.trim(), cloud_provider_name(input.provider), has_token, Utc::now().to_rfc3339()],
        )?;
        self.get_cloud_account(id)
    }

    pub fn touch_cloud_account(&self, id: &str) -> AppResult<()> {
        self.connection()?.execute(
            "UPDATE cloud_accounts SET last_synced_at = ?2 WHERE id = ?1",
            params![id, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn ensure_cloud_account_deletable(&self, id: &str) -> AppResult<()> {
        let linked = self.connection()?.query_row(
            "SELECT COUNT(*) FROM servers WHERE cloud_account_id = ?1",
            [id],
            |row| row.get::<_, i64>(0),
        )?;
        if linked > 0 {
            return Err(AppError::Message(format!(
                "还有 {linked} 台本地 VPS 关联这个云账户，请先在资产页解除关联或删除这些 VPS"
            )));
        }
        Ok(())
    }

    pub fn delete_cloud_account(&self, id: &str) -> AppResult<()> {
        self.connection()?
            .execute("DELETE FROM cloud_accounts WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn find_cloud_server(
        &self,
        account_id: &str,
        resource_id: &str,
    ) -> AppResult<Option<Server>> {
        let connection = self.connection()?;
        let found = connection
            .query_row(
                "SELECT id FROM servers WHERE cloud_account_id = ?1 AND cloud_resource_id = ?2",
                params![account_id, resource_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        drop(connection);
        found.map(|id| self.get_server(&id)).transpose()
    }

    pub fn list_auth_profiles(&self) -> AppResult<Vec<AuthProfile>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, name, auth_method, private_key_path, has_password, has_key_passphrase, created_at, updated_at
             FROM auth_profiles ORDER BY name COLLATE NOCASE",
        )?;
        let rows = statement.query_map([], row_to_auth_profile)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_auth_profile(&self, id: &str) -> AppResult<AuthProfile> {
        self.connection()?
            .query_row(
                "SELECT id, name, auth_method, private_key_path, has_password, has_key_passphrase, created_at, updated_at
                 FROM auth_profiles WHERE id = ?1",
                [id],
                row_to_auth_profile,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => AppError::Message("未找到这个认证身份".into()),
                other => other.into(),
            })
    }

    pub fn create_auth_profile(&self, input: &AuthProfileInput) -> AppResult<AuthProfile> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        self.connection()?.execute(
            "INSERT INTO auth_profiles (
                id, name, auth_method, private_key_path, has_password, has_key_passphrase, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id,
                input.name.trim(),
                auth_name(&input.auth_method),
                clean(input.private_key_path.as_deref()),
                input.password.as_deref().is_some_and(|value| !value.is_empty()),
                input.key_passphrase.as_deref().is_some_and(|value| !value.is_empty()),
                now,
                now,
            ],
        )?;
        self.get_auth_profile(&id)
    }

    pub fn update_auth_profile(
        &self,
        id: &str,
        input: &AuthProfileInput,
    ) -> AppResult<AuthProfile> {
        let existing = self.get_auth_profile(id)?;
        let has_password = match input.password.as_deref() {
            Some(value) if !value.is_empty() => true,
            Some(_) => false,
            None => existing.has_password,
        };
        let has_key_passphrase = match input.key_passphrase.as_deref() {
            Some(value) if !value.is_empty() => true,
            Some(_) => false,
            None => existing.has_key_passphrase,
        };
        self.connection()?.execute(
            "UPDATE auth_profiles SET name = ?2, auth_method = ?3, private_key_path = ?4,
                has_password = ?5, has_key_passphrase = ?6, updated_at = ?7 WHERE id = ?1",
            params![
                id,
                input.name.trim(),
                auth_name(&input.auth_method),
                clean(input.private_key_path.as_deref()),
                has_password,
                has_key_passphrase,
                Utc::now().to_rfc3339(),
            ],
        )?;
        self.get_auth_profile(id)
    }

    pub fn ensure_auth_profile_deletable(&self, id: &str) -> AppResult<()> {
        let connection = self.connection()?;
        let groups = connection.query_row(
            "SELECT COUNT(*) FROM server_groups WHERE auth_profile_id = ?1",
            [id],
            |row| row.get::<_, i64>(0),
        )?;
        let servers = connection.query_row(
            "SELECT COUNT(*) FROM servers WHERE auth_profile_id = ?1",
            [id],
            |row| row.get::<_, i64>(0),
        )?;
        if groups > 0 || servers > 0 {
            return Err(AppError::Message(format!(
                "这个认证身份仍被 {groups} 个分组和 {servers} 台 VPS 使用"
            )));
        }
        Ok(())
    }

    pub fn delete_auth_profile(&self, id: &str) -> AppResult<()> {
        self.connection()?
            .execute("DELETE FROM auth_profiles WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn list_server_groups(&self) -> AppResult<Vec<ServerGroup>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, name, jump_server_id, auth_profile_id, default_port, default_username, created_at, updated_at
             FROM server_groups ORDER BY name COLLATE NOCASE",
        )?;
        let rows = statement.query_map([], row_to_server_group)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_server_group_by_name(&self, name: &str) -> AppResult<ServerGroup> {
        self.connection()?
            .query_row(
                "SELECT id, name, jump_server_id, auth_profile_id, default_port, default_username, created_at, updated_at
                 FROM server_groups WHERE name = ?1 COLLATE NOCASE",
                [name],
                row_to_server_group,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => AppError::Message("所选分组还没有默认连接设置".into()),
                other => other.into(),
            })
    }

    pub fn create_server_group(&self, input: &ServerGroupInput) -> AppResult<ServerGroup> {
        validate_group(self, input)?;
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        self.connection()?.execute(
            "INSERT INTO server_groups (
                id, name, jump_server_id, auth_profile_id, default_port, default_username, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id,
                input.name.trim(),
                clean(input.jump_server_id.as_deref()),
                clean(input.auth_profile_id.as_deref()),
                input.default_port,
                clean(input.default_username.as_deref()),
                now,
                now,
            ],
        ).map_err(unique_group_error)?;
        self.get_server_group_by_name(input.name.trim())
    }

    pub fn update_server_group(
        &self,
        id: &str,
        input: &ServerGroupInput,
    ) -> AppResult<ServerGroup> {
        validate_group(self, input)?;
        let new_name = input.name.trim();
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let old_name: String = tx
            .query_row(
                "SELECT name FROM server_groups WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => AppError::Message("未找到这个分组".into()),
                other => other.into(),
            })?;
        tx.execute(
            "UPDATE server_groups SET name = ?2, jump_server_id = ?3, auth_profile_id = ?4,
                default_port = ?5, default_username = ?6, updated_at = ?7 WHERE id = ?1",
            params![
                id,
                new_name,
                clean(input.jump_server_id.as_deref()),
                clean(input.auth_profile_id.as_deref()),
                input.default_port,
                clean(input.default_username.as_deref()),
                Utc::now().to_rfc3339(),
            ],
        )
        .map_err(unique_group_error)?;
        if !old_name.eq_ignore_ascii_case(new_name) || old_name != new_name {
            tx.execute(
                "UPDATE servers SET group_name = ?2 WHERE group_name = ?1 COLLATE NOCASE",
                params![old_name, new_name],
            )?;
        }
        tx.commit()?;
        drop(connection);
        self.get_server_group_by_name(new_name)
    }

    pub fn delete_server_group(&self, id: &str) -> AppResult<()> {
        self.connection()?
            .execute("DELETE FROM server_groups WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn log_operation(
        &self,
        server_id: &str,
        action: &str,
        success: bool,
        summary: &str,
    ) -> AppResult<()> {
        self.connection()?.execute(
            "INSERT INTO operation_logs (server_id, action, success, summary, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![server_id, action, success, summary, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn list_operation_logs(&self, server_id: Option<&str>) -> AppResult<Vec<OperationLog>> {
        let connection = self.connection()?;
        let sql = "SELECT l.id, l.server_id, s.name, l.action, l.success, l.summary, l.created_at
                   FROM operation_logs l JOIN servers s ON s.id = l.server_id";
        let mut logs = Vec::new();
        if let Some(id) = server_id {
            let mut statement = connection.prepare(&format!(
                "{sql} WHERE l.server_id = ?1 ORDER BY l.id DESC LIMIT 100"
            ))?;
            let rows = statement.query_map([id], row_to_log)?;
            for row in rows {
                logs.push(row?);
            }
        } else {
            let mut statement =
                connection.prepare(&format!("{sql} ORDER BY l.id DESC LIMIT 100"))?;
            let rows = statement.query_map([], row_to_log)?;
            for row in rows {
                logs.push(row?);
            }
        }
        Ok(logs)
    }

    pub fn list_snippets(&self) -> AppResult<Vec<Snippet>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, name, description, command, created_at, updated_at
             FROM snippets ORDER BY name COLLATE NOCASE, created_at",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(Snippet {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                command: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn create_snippet(&self, input: &SnippetInput) -> AppResult<Snippet> {
        validate_snippet(input)?;
        let snippet = Snippet {
            id: Uuid::new_v4().to_string(),
            name: input.name.trim().to_string(),
            description: clean(input.description.as_deref()),
            command: input.command.clone(),
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        };
        self.connection()?.execute(
            "INSERT INTO snippets (id, name, description, command, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                snippet.id,
                snippet.name,
                snippet.description,
                snippet.command,
                snippet.created_at,
                snippet.updated_at
            ],
        )?;
        Ok(snippet)
    }

    pub fn update_snippet(&self, id: &str, input: &SnippetInput) -> AppResult<Snippet> {
        validate_snippet(input)?;
        let changed = self.connection()?.execute(
            "UPDATE snippets SET name = ?2, description = ?3, command = ?4, updated_at = ?5
             WHERE id = ?1",
            params![
                id,
                input.name.trim(),
                clean(input.description.as_deref()),
                input.command,
                Utc::now().to_rfc3339()
            ],
        )?;
        if changed == 0 {
            return Err(AppError::Message("未找到这个命令片段".into()));
        }
        self.list_snippets()?
            .into_iter()
            .find(|snippet| snippet.id == id)
            .ok_or_else(|| AppError::Message("未找到这个命令片段".into()))
    }

    pub fn delete_snippet(&self, id: &str) -> AppResult<()> {
        self.connection()?
            .execute("DELETE FROM snippets WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn list_runbooks(&self) -> AppResult<Vec<Runbook>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, name, description, steps, created_at, updated_at
             FROM runbooks ORDER BY name COLLATE NOCASE, created_at",
        )?;
        let rows = statement.query_map([], row_to_runbook)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_runbook(&self, id: &str) -> AppResult<Runbook> {
        self.connection()?
            .query_row(
                "SELECT id, name, description, steps, created_at, updated_at
                 FROM runbooks WHERE id = ?1",
                [id],
                row_to_runbook,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    AppError::Message("未找到这个 Runbook".into())
                }
                other => other.into(),
            })
    }

    pub fn create_runbook(&self, input: &RunbookInput) -> AppResult<Runbook> {
        validate_runbook(input)?;
        let now = Utc::now().to_rfc3339();
        let runbook = Runbook {
            id: Uuid::new_v4().to_string(),
            name: input.name.trim().to_string(),
            description: clean(input.description.as_deref()),
            steps: normalized_steps(input),
            created_at: now.clone(),
            updated_at: now,
        };
        self.connection()?.execute(
            "INSERT INTO runbooks (id, name, description, steps, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                runbook.id,
                runbook.name,
                runbook.description,
                serde_json::to_string(&runbook.steps)
                    .map_err(|error| AppError::Message(error.to_string()))?,
                runbook.created_at,
                runbook.updated_at,
            ],
        )?;
        Ok(runbook)
    }

    pub fn update_runbook(&self, id: &str, input: &RunbookInput) -> AppResult<Runbook> {
        validate_runbook(input)?;
        let existing = self.get_runbook(id)?;
        let runbook = Runbook {
            id: existing.id,
            name: input.name.trim().to_string(),
            description: clean(input.description.as_deref()),
            steps: normalized_steps(input),
            created_at: existing.created_at,
            updated_at: Utc::now().to_rfc3339(),
        };
        self.connection()?.execute(
            "UPDATE runbooks SET name = ?2, description = ?3, steps = ?4, updated_at = ?5
             WHERE id = ?1",
            params![
                runbook.id,
                runbook.name,
                runbook.description,
                serde_json::to_string(&runbook.steps)
                    .map_err(|error| AppError::Message(error.to_string()))?,
                runbook.updated_at,
            ],
        )?;
        Ok(runbook)
    }

    pub fn delete_runbook(&self, id: &str) -> AppResult<()> {
        self.connection()?
            .execute("DELETE FROM runbooks WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn list_runbook_schedules(&self) -> AppResult<Vec<RunbookSchedule>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT sc.id, sc.runbook_id, r.name, sc.server_ids, sc.interval_minutes,
                    sc.enabled, sc.concurrency, sc.canary_count, sc.timeout_seconds,
                    sc.next_run_at, sc.last_run_at, sc.last_execution_id, sc.last_error,
                    sc.created_at, sc.updated_at
             FROM runbook_schedules sc JOIN runbooks r ON r.id = sc.runbook_id
             ORDER BY r.name COLLATE NOCASE, sc.created_at",
        )?;
        let rows = statement.query_map([], row_to_runbook_schedule)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_runbook_schedule(&self, id: &str) -> AppResult<RunbookSchedule> {
        self.connection()?
            .query_row(
                "SELECT sc.id, sc.runbook_id, r.name, sc.server_ids, sc.interval_minutes,
                        sc.enabled, sc.concurrency, sc.canary_count, sc.timeout_seconds,
                        sc.next_run_at, sc.last_run_at, sc.last_execution_id, sc.last_error,
                        sc.created_at, sc.updated_at
                 FROM runbook_schedules sc JOIN runbooks r ON r.id = sc.runbook_id
                 WHERE sc.id = ?1",
                [id],
                row_to_runbook_schedule,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    AppError::Message("未找到这个计划任务".into())
                }
                other => other.into(),
            })
    }

    pub fn create_runbook_schedule(
        &self,
        input: &RunbookScheduleInput,
    ) -> AppResult<RunbookSchedule> {
        self.validate_runbook_schedule(input)?;
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let next_run_at = input
            .enabled
            .then(|| (now + Duration::minutes(input.interval_minutes.into())).to_rfc3339());
        self.connection()?.execute(
            "INSERT INTO runbook_schedules (
                id, runbook_id, server_ids, interval_minutes, enabled, concurrency,
                canary_count, timeout_seconds, next_run_at, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                id,
                input.runbook_id,
                json_string(&input.server_ids)?,
                input.interval_minutes,
                input.enabled,
                input.concurrency,
                input.canary_count,
                input.timeout_seconds,
                next_run_at,
                now.to_rfc3339(),
                now.to_rfc3339(),
            ],
        )?;
        self.get_runbook_schedule(&id)
    }

    pub fn update_runbook_schedule(
        &self,
        id: &str,
        input: &RunbookScheduleInput,
    ) -> AppResult<RunbookSchedule> {
        self.get_runbook_schedule(id)?;
        self.validate_runbook_schedule(input)?;
        let now = Utc::now();
        let next_run_at = input
            .enabled
            .then(|| (now + Duration::minutes(input.interval_minutes.into())).to_rfc3339());
        self.connection()?.execute(
            "UPDATE runbook_schedules SET runbook_id = ?2, server_ids = ?3,
                interval_minutes = ?4, enabled = ?5, concurrency = ?6,
                canary_count = ?7, timeout_seconds = ?8, next_run_at = ?9,
                last_error = NULL, updated_at = ?10
             WHERE id = ?1",
            params![
                id,
                input.runbook_id,
                json_string(&input.server_ids)?,
                input.interval_minutes,
                input.enabled,
                input.concurrency,
                input.canary_count,
                input.timeout_seconds,
                next_run_at,
                now.to_rfc3339(),
            ],
        )?;
        self.get_runbook_schedule(id)
    }

    pub fn set_runbook_schedule_enabled(
        &self,
        id: &str,
        enabled: bool,
    ) -> AppResult<RunbookSchedule> {
        let schedule = self.get_runbook_schedule(id)?;
        let now = Utc::now();
        let next_run_at = enabled
            .then(|| (now + Duration::minutes(schedule.interval_minutes.into())).to_rfc3339());
        self.connection()?.execute(
            "UPDATE runbook_schedules SET enabled = ?2, next_run_at = ?3,
                last_error = NULL, updated_at = ?4 WHERE id = ?1",
            params![id, enabled, next_run_at, now.to_rfc3339()],
        )?;
        self.get_runbook_schedule(id)
    }

    pub fn delete_runbook_schedule(&self, id: &str) -> AppResult<()> {
        self.connection()?
            .execute("DELETE FROM runbook_schedules WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn due_runbook_schedules(&self) -> AppResult<Vec<RunbookSchedule>> {
        let now = Utc::now().to_rfc3339();
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT sc.id, sc.runbook_id, r.name, sc.server_ids, sc.interval_minutes,
                    sc.enabled, sc.concurrency, sc.canary_count, sc.timeout_seconds,
                    sc.next_run_at, sc.last_run_at, sc.last_execution_id, sc.last_error,
                    sc.created_at, sc.updated_at
             FROM runbook_schedules sc JOIN runbooks r ON r.id = sc.runbook_id
             WHERE sc.enabled = 1 AND sc.next_run_at <= ?1 ORDER BY sc.next_run_at",
        )?;
        let rows = statement.query_map([now], row_to_runbook_schedule)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn mark_runbook_schedule_started(&self, id: &str, execution_id: &str) -> AppResult<()> {
        let schedule = self.get_runbook_schedule(id)?;
        let now = Utc::now();
        let next = now + Duration::minutes(schedule.interval_minutes.into());
        self.connection()?.execute(
            "UPDATE runbook_schedules SET last_run_at = ?2, next_run_at = ?3,
                last_execution_id = ?4, last_error = NULL, updated_at = ?2 WHERE id = ?1",
            params![id, now.to_rfc3339(), next.to_rfc3339(), execution_id],
        )?;
        Ok(())
    }

    pub fn mark_runbook_schedule_error(&self, id: &str, error: &str) -> AppResult<()> {
        let schedule = self.get_runbook_schedule(id)?;
        let now = Utc::now();
        let next = now + Duration::minutes(schedule.interval_minutes.into());
        self.connection()?.execute(
            "UPDATE runbook_schedules SET last_run_at = ?2, next_run_at = ?3,
                last_error = ?4, updated_at = ?2 WHERE id = ?1",
            params![id, now.to_rfc3339(), next.to_rfc3339(), error],
        )?;
        Ok(())
    }

    pub fn create_runbook_execution(
        &self,
        request: &RunbookExecutionRequest,
    ) -> AppResult<RunbookExecution> {
        let id = Uuid::new_v4().to_string();
        self.connection()?.execute(
            "INSERT INTO runbook_executions (
                id, runbook_id, runbook_name, runbook_snapshot, schedule_id, retry_of,
                trigger_kind, status, target_ids, concurrency, canary_count, timeout_seconds,
                created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'queued', ?8, ?9, ?10, ?11, ?12)",
            params![
                id,
                request.runbook.id,
                request.runbook.name,
                serde_json::to_string(&request.runbook)
                    .map_err(|error| AppError::Message(error.to_string()))?,
                request.schedule_id,
                request.retry_of,
                runbook_trigger(request.trigger),
                json_string(&request.target_ids)?,
                request.concurrency,
                request.canary_count,
                request.timeout_seconds,
                Utc::now().to_rfc3339(),
            ],
        )?;
        self.get_runbook_execution(&id)
    }

    pub fn mark_runbook_execution_running(&self, id: &str) -> AppResult<()> {
        self.connection()?.execute(
            "UPDATE runbook_executions SET status = 'running', started_at = ?2
             WHERE id = ?1 AND status = 'queued'",
            params![id, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn finish_runbook_execution(
        &self,
        id: &str,
        status: RunbookExecutionStatus,
        result: Option<&RunbookResult>,
        error: Option<&str>,
    ) -> AppResult<()> {
        self.connection()?.execute(
            "UPDATE runbook_executions SET status = ?2, success_count = ?3,
                failure_count = ?4, skipped_count = ?5, finished_at = ?6,
                result = ?7, error = ?8 WHERE id = ?1",
            params![
                id,
                runbook_execution_status(status),
                result.map_or(0, |value| value.success_count),
                result.map_or(0, |value| value.failure_count),
                result.map_or(0, |value| value.skipped_count),
                Utc::now().to_rfc3339(),
                result
                    .map(serde_json::to_string)
                    .transpose()
                    .map_err(|error| AppError::Message(error.to_string()))?,
                error,
            ],
        )?;
        Ok(())
    }

    pub fn recover_interrupted_runbook_executions(&self) -> AppResult<usize> {
        let count = self.connection()?.execute(
            "UPDATE runbook_executions SET status = 'interrupted', finished_at = ?1,
                error = '应用退出导致本地执行中断；远程命令的最后状态需要人工核对'
             WHERE status IN ('queued', 'running')",
            [Utc::now().to_rfc3339()],
        )?;
        Ok(count)
    }

    pub fn list_runbook_executions(
        &self,
        runbook_id: Option<&str>,
    ) -> AppResult<Vec<RunbookExecution>> {
        let connection = self.connection()?;
        let select = "SELECT id, runbook_id, runbook_name, schedule_id, retry_of,
            trigger_kind, status, target_ids, concurrency, canary_count, timeout_seconds,
            success_count, failure_count, skipped_count, created_at, started_at,
            finished_at, NULL, error FROM runbook_executions";
        let mut executions = Vec::new();
        if let Some(runbook_id) = runbook_id {
            let mut statement = connection.prepare(&format!(
                "{select} WHERE runbook_id = ?1 ORDER BY created_at DESC LIMIT 100"
            ))?;
            let rows = statement.query_map([runbook_id], row_to_runbook_execution)?;
            for row in rows {
                executions.push(row?);
            }
        } else {
            let mut statement =
                connection.prepare(&format!("{select} ORDER BY created_at DESC LIMIT 100"))?;
            let rows = statement.query_map([], row_to_runbook_execution)?;
            for row in rows {
                executions.push(row?);
            }
        }
        Ok(executions)
    }

    pub fn get_runbook_execution(&self, id: &str) -> AppResult<RunbookExecution> {
        self.connection()?
            .query_row(
                "SELECT id, runbook_id, runbook_name, schedule_id, retry_of,
                    trigger_kind, status, target_ids, concurrency, canary_count,
                    timeout_seconds, success_count, failure_count, skipped_count,
                    created_at, started_at, finished_at, result, error
                 FROM runbook_executions WHERE id = ?1",
                [id],
                row_to_runbook_execution,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    AppError::Message("未找到这条 Runbook 执行记录".into())
                }
                other => other.into(),
            })
    }

    pub fn runbook_retry_request(&self, id: &str) -> AppResult<RunbookExecutionRequest> {
        let execution = self.get_runbook_execution(id)?;
        if matches!(
            execution.status,
            RunbookExecutionStatus::Queued | RunbookExecutionStatus::Running
        ) {
            return Err(AppError::Message("运行中的任务不能重试".into()));
        }
        let connection = self.connection()?;
        let snapshot: String = connection.query_row(
            "SELECT runbook_snapshot FROM runbook_executions WHERE id = ?1",
            [id],
            |row| row.get(0),
        )?;
        drop(connection);
        let runbook: Runbook = serde_json::from_str(&snapshot)
            .map_err(|_| AppError::Message("历史 Runbook 快照无法读取".into()))?;
        let successful = execution
            .result
            .as_ref()
            .map(|result| {
                result
                    .results
                    .iter()
                    .filter(|item| item.success)
                    .map(|item| item.server_id.as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let target_ids = execution
            .target_ids
            .iter()
            .filter(|target| !successful.contains(&target.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if target_ids.is_empty() {
            return Err(AppError::Message("这条记录没有可重试的失败目标".into()));
        }
        Ok(RunbookExecutionRequest {
            runbook,
            target_ids,
            concurrency: execution.concurrency,
            canary_count: execution.canary_count,
            timeout_seconds: execution.timeout_seconds,
            trigger: RunbookTrigger::Retry,
            schedule_id: None,
            retry_of: Some(execution.id),
        })
    }

    fn validate_runbook_schedule(&self, input: &RunbookScheduleInput) -> AppResult<()> {
        self.get_runbook(&input.runbook_id)?;
        let mut ids = input.server_ids.clone();
        ids.sort();
        ids.dedup();
        if ids.len() != input.server_ids.len() || ids.is_empty() || ids.len() > 50 {
            return Err(AppError::Message(
                "计划任务需要 1 到 50 台不重复的服务器".into(),
            ));
        }
        for id in ids {
            self.get_server(&id)?;
        }
        if !(1..=10_080).contains(&input.interval_minutes) {
            return Err(AppError::Message("执行间隔必须在 1 分钟到 7 天之间".into()));
        }
        if !(1..=10).contains(&input.concurrency)
            || !(10..=86_400).contains(&input.timeout_seconds)
            || input.canary_count > 10
            || input.canary_count >= input.server_ids.len()
        {
            return Err(AppError::Message(
                "并发数需为 1 到 10，单步超时时长需为 10 秒到 24 小时，灰度数量必须小于目标数"
                    .into(),
            ));
        }
        Ok(())
    }

    pub fn list_data_protection_policies(
        &self,
        server_id: &str,
    ) -> AppResult<Vec<DataProtectionPolicy>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, server_id, name, repository_kind, repository, source_kind, paths,
                    volume_name, database_container, database_name, database_user, keep_last,
                    has_repository_password, has_s3_credentials, has_database_password,
                    schedule_enabled, schedule_interval_minutes, next_run_at, last_run_at,
                    last_status, last_error, created_at, updated_at
             FROM data_protection_policies WHERE server_id = ?1
             ORDER BY name COLLATE NOCASE, created_at",
        )?;
        let rows = statement.query_map([server_id], row_to_data_protection_policy)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_data_protection_policy(&self, id: &str) -> AppResult<DataProtectionPolicy> {
        self.connection()?
            .query_row(
                "SELECT id, server_id, name, repository_kind, repository, source_kind, paths,
                        volume_name, database_container, database_name, database_user, keep_last,
                        has_repository_password, has_s3_credentials, has_database_password,
                        schedule_enabled, schedule_interval_minutes, next_run_at, last_run_at,
                        last_status, last_error, created_at, updated_at
                 FROM data_protection_policies WHERE id = ?1",
                [id],
                row_to_data_protection_policy,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    AppError::Message("未找到这个数据保护策略".into())
                }
                other => other.into(),
            })
    }

    pub fn create_data_protection_policy(
        &self,
        id: &str,
        server_id: &str,
        input: &DataProtectionInput,
    ) -> AppResult<DataProtectionPolicy> {
        self.get_server(server_id)?;
        validate_data_protection(input, false, false)?;
        let interval_minutes = validate_backup_schedule_interval(
            input.schedule_enabled,
            input.schedule_interval_minutes,
        )?;
        let now_dt = Utc::now();
        let now = now_dt.to_rfc3339();
        let next_run_at = input
            .schedule_enabled
            .then(|| (now_dt + Duration::minutes(i64::from(interval_minutes))).to_rfc3339());
        self.connection()?.execute(
            "INSERT INTO data_protection_policies (
                id, server_id, name, repository_kind, repository, source_kind, paths,
                volume_name, database_container, database_name, database_user, keep_last,
                has_repository_password, has_s3_credentials, has_database_password,
                schedule_enabled, schedule_interval_minutes, next_run_at,
                created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 1, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                id,
                server_id,
                input.name.trim(),
                restic_repository_kind(input.repository_kind),
                input.repository.trim(),
                protection_source_kind(input.source_kind),
                json_string(&input.paths)?,
                clean(input.volume_name.as_deref()),
                clean(input.database_container.as_deref()),
                clean(input.database_name.as_deref()),
                clean(input.database_user.as_deref()),
                input.keep_last,
                input
                    .s3_access_key_id
                    .as_deref()
                    .is_some_and(|value| !value.is_empty())
                    && input
                        .s3_secret_access_key
                        .as_deref()
                        .is_some_and(|value| !value.is_empty()),
                input
                    .database_password
                    .as_deref()
                    .is_some_and(|value| !value.is_empty()),
                input.schedule_enabled,
                interval_minutes,
                next_run_at,
                now,
                now,
            ],
        )?;
        self.get_data_protection_policy(id)
    }

    pub fn update_data_protection_policy(
        &self,
        id: &str,
        input: &DataProtectionInput,
    ) -> AppResult<DataProtectionPolicy> {
        let existing = self.get_data_protection_policy(id)?;
        validate_data_protection(
            input,
            existing.has_repository_password,
            existing.has_s3_credentials,
        )?;
        let interval_minutes = validate_backup_schedule_interval(
            input.schedule_enabled,
            input.schedule_interval_minutes,
        )?;
        let now_dt = Utc::now();
        let next_run_at = if !input.schedule_enabled {
            None
        } else if existing.schedule_enabled
            && existing.schedule_interval_minutes == interval_minutes
            && existing.next_run_at.is_some()
        {
            existing.next_run_at.clone()
        } else {
            Some((now_dt + Duration::minutes(i64::from(interval_minutes))).to_rfc3339())
        };
        let has_s3_credentials = input.repository_kind == ResticRepositoryKind::S3
            && (existing.has_s3_credentials
                || match (
                    input.s3_access_key_id.as_deref(),
                    input.s3_secret_access_key.as_deref(),
                ) {
                    (Some(access), Some(secret)) => !access.is_empty() && !secret.is_empty(),
                    _ => false,
                });
        let has_database_password = input
            .database_password
            .as_deref()
            .map(|value| !value.is_empty())
            .unwrap_or(existing.has_database_password);
        self.connection()?.execute(
            "UPDATE data_protection_policies SET name = ?2, repository_kind = ?3,
                repository = ?4, source_kind = ?5, paths = ?6, volume_name = ?7,
                database_container = ?8, database_name = ?9, database_user = ?10,
                keep_last = ?11, has_repository_password = ?12,
                has_s3_credentials = ?13, has_database_password = ?14,
                schedule_enabled = ?15, schedule_interval_minutes = ?16, next_run_at = ?17,
                updated_at = ?18
             WHERE id = ?1",
            params![
                id,
                input.name.trim(),
                restic_repository_kind(input.repository_kind),
                input.repository.trim(),
                protection_source_kind(input.source_kind),
                json_string(&input.paths)?,
                clean(input.volume_name.as_deref()),
                clean(input.database_container.as_deref()),
                clean(input.database_name.as_deref()),
                clean(input.database_user.as_deref()),
                input.keep_last,
                input
                    .repository_password
                    .as_deref()
                    .is_some_and(|value| !value.is_empty())
                    || existing.has_repository_password,
                has_s3_credentials,
                has_database_password,
                input.schedule_enabled,
                interval_minutes,
                next_run_at,
                now_dt.to_rfc3339(),
            ],
        )?;
        self.get_data_protection_policy(id)
    }

    pub fn due_data_protection_policies(&self) -> AppResult<Vec<DataProtectionPolicy>> {
        let now = Utc::now().to_rfc3339();
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, server_id, name, repository_kind, repository, source_kind, paths,
                    volume_name, database_container, database_name, database_user, keep_last,
                    has_repository_password, has_s3_credentials, has_database_password,
                    schedule_enabled, schedule_interval_minutes, next_run_at, last_run_at,
                    last_status, last_error, created_at, updated_at
             FROM data_protection_policies
             WHERE schedule_enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?1
             ORDER BY next_run_at",
        )?;
        let rows = statement.query_map([now], row_to_data_protection_policy)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn mark_data_protection_scheduled_started(&self, id: &str) -> AppResult<()> {
        let policy = self.get_data_protection_policy(id)?;
        let now = Utc::now();
        let next = now + Duration::minutes(i64::from(policy.schedule_interval_minutes.max(60)));
        self.connection()?.execute(
            "UPDATE data_protection_policies
             SET next_run_at = ?2, last_status = 'running', last_error = NULL, updated_at = ?3
             WHERE id = ?1",
            params![id, next.to_rfc3339(), now.to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn mark_data_protection_scheduled_result(
        &self,
        id: &str,
        succeeded: bool,
        error: Option<&str>,
    ) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();
        self.connection()?.execute(
            "UPDATE data_protection_policies
             SET last_run_at = ?2, last_status = ?3, last_error = ?4, updated_at = ?2
             WHERE id = ?1",
            params![
                id,
                now,
                if succeeded { "succeeded" } else { "failed" },
                error
            ],
        )?;
        Ok(())
    }

    pub fn delete_data_protection_policy(&self, id: &str) -> AppResult<()> {
        self.connection()?
            .execute("DELETE FROM data_protection_policies WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn start_data_protection_run(
        &self,
        policy_id: &str,
        action: DataProtectionAction,
        target: Option<&str>,
    ) -> AppResult<DataProtectionRun> {
        let id = Uuid::new_v4().to_string();
        self.connection()?.execute(
            "INSERT INTO data_protection_runs (
                id, policy_id, action, status, target, started_at
             ) VALUES (?1, ?2, ?3, 'running', ?4, ?5)",
            params![
                id,
                policy_id,
                data_protection_action(action),
                target,
                Utc::now().to_rfc3339()
            ],
        )?;
        self.get_data_protection_run(&id)
    }

    pub fn finish_data_protection_run(
        &self,
        id: &str,
        status: DataProtectionRunStatus,
        snapshot_id: Option<&str>,
        output: &str,
        error: Option<&str>,
        duration_ms: u64,
    ) -> AppResult<()> {
        self.connection()?.execute(
            "UPDATE data_protection_runs SET status = ?2, snapshot_id = ?3,
                output = ?4, error = ?5, finished_at = ?6, duration_ms = ?7 WHERE id = ?1",
            params![
                id,
                data_protection_run_status(status),
                snapshot_id,
                output,
                error,
                Utc::now().to_rfc3339(),
                duration_ms as i64,
            ],
        )?;
        Ok(())
    }

    pub fn recover_interrupted_data_protection_runs(&self) -> AppResult<usize> {
        let count = self.connection()?.execute(
            "UPDATE data_protection_runs SET status = 'failed', finished_at = ?1,
                error = '应用退出导致本地备份任务中断；请检查 Restic 仓库状态'
             WHERE status = 'running'",
            [Utc::now().to_rfc3339()],
        )?;
        Ok(count)
    }

    pub fn get_data_protection_run(&self, id: &str) -> AppResult<DataProtectionRun> {
        self.connection()?
            .query_row(
                "SELECT id, policy_id, action, status, snapshot_id, target, output, error,
                        started_at, finished_at, duration_ms
                 FROM data_protection_runs WHERE id = ?1",
                [id],
                row_to_data_protection_run,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    AppError::Message("未找到这条数据保护记录".into())
                }
                other => other.into(),
            })
    }

    pub fn list_data_protection_runs(&self, policy_id: &str) -> AppResult<Vec<DataProtectionRun>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, policy_id, action, status, snapshot_id, target, output, error,
                    started_at, finished_at, duration_ms
             FROM data_protection_runs WHERE policy_id = ?1
             ORDER BY started_at DESC LIMIT 100",
        )?;
        let rows = statement.query_map([policy_id], row_to_data_protection_run)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn list_port_forwards(&self) -> AppResult<Vec<PortForwardRule>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, name, server_id, kind, listen_address, listen_port, destination_host,
                    destination_port, created_at, updated_at
             FROM port_forwards ORDER BY name COLLATE NOCASE, created_at",
        )?;
        let rows = statement.query_map([], row_to_port_forward)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_port_forward(&self, id: &str) -> AppResult<PortForwardRule> {
        self.connection()?
            .query_row(
                "SELECT id, name, server_id, kind, listen_address, listen_port, destination_host,
                        destination_port, created_at, updated_at
                 FROM port_forwards WHERE id = ?1",
                [id],
                row_to_port_forward,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    AppError::Message("未找到这条端口转发规则".into())
                }
                other => other.into(),
            })
    }

    pub fn create_port_forward(&self, input: &PortForwardInput) -> AppResult<PortForwardRule> {
        validate_port_forward(input)?;
        self.get_server(&input.server_id)?;
        let rule = PortForwardRule {
            id: Uuid::new_v4().to_string(),
            name: input.name.trim().to_string(),
            server_id: input.server_id.clone(),
            kind: input.kind,
            listen_address: input.listen_address.trim().to_string(),
            listen_port: input.listen_port,
            destination_host: input.destination_host.trim().to_string(),
            destination_port: input.destination_port,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        };
        self.connection()?.execute(
            "INSERT INTO port_forwards (
                id, name, server_id, kind, listen_address, listen_port, destination_host,
                destination_port, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                rule.id,
                rule.name,
                rule.server_id,
                forward_kind(rule.kind),
                rule.listen_address,
                rule.listen_port,
                rule.destination_host,
                rule.destination_port,
                rule.created_at,
                rule.updated_at
            ],
        )?;
        Ok(rule)
    }

    pub fn delete_port_forward(&self, id: &str) -> AppResult<()> {
        self.connection()?
            .execute("DELETE FROM port_forwards WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn list_service_monitors(&self) -> AppResult<Vec<ServiceMonitorView>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, name, kind, target, port, server_id, interval_seconds, timeout_seconds,
                    expected_status, keyword, failure_threshold, certificate_warning_days,
                    enabled, created_at, updated_at
             FROM service_monitors ORDER BY name COLLATE NOCASE, created_at",
        )?;
        let monitors = statement
            .query_map([], row_to_service_monitor)?
            .collect::<Result<Vec<_>, _>>()?;
        monitors
            .iter()
            .map(|monitor| service_monitor_view(&connection, monitor).map_err(Into::into))
            .collect()
    }

    pub fn get_service_monitor(&self, id: &str) -> AppResult<ServiceMonitor> {
        self.connection()?
            .query_row(
                "SELECT id, name, kind, target, port, server_id, interval_seconds, timeout_seconds,
                        expected_status, keyword, failure_threshold, certificate_warning_days,
                        enabled, created_at, updated_at
                 FROM service_monitors WHERE id = ?1",
                [id],
                row_to_service_monitor,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    AppError::Message("未找到这个服务监控".into())
                }
                other => other.into(),
            })
    }

    pub fn create_service_monitor(
        &self,
        input: &ServiceMonitorInput,
    ) -> AppResult<ServiceMonitorView> {
        crate::service_monitor::validate_input(input)?;
        if let Some(server_id) = clean(input.server_id.as_deref()) {
            self.get_server(&server_id)?;
        }
        let monitor = ServiceMonitor {
            id: Uuid::new_v4().to_string(),
            name: input.name.trim().to_string(),
            kind: input.kind,
            target: input.target.trim().to_string(),
            port: input.port.filter(|port| *port != 0),
            server_id: clean(input.server_id.as_deref()),
            interval_seconds: input.interval_seconds,
            timeout_seconds: input.timeout_seconds,
            expected_status: input.expected_status,
            keyword: clean(input.keyword.as_deref()),
            failure_threshold: input.failure_threshold,
            certificate_warning_days: input.certificate_warning_days,
            enabled: input.enabled,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        };
        self.connection()?.execute(
            "INSERT INTO service_monitors (
                id, name, kind, target, port, server_id, interval_seconds, timeout_seconds,
                expected_status, keyword, failure_threshold, certificate_warning_days,
                enabled, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                monitor.id,
                monitor.name,
                service_monitor_kind(monitor.kind),
                monitor.target,
                monitor.port,
                monitor.server_id,
                monitor.interval_seconds,
                monitor.timeout_seconds,
                monitor.expected_status,
                monitor.keyword,
                monitor.failure_threshold,
                monitor.certificate_warning_days,
                monitor.enabled,
                monitor.created_at,
                monitor.updated_at,
            ],
        )?;
        self.service_monitor_view(&monitor.id)
    }

    pub fn update_service_monitor(
        &self,
        id: &str,
        input: &ServiceMonitorInput,
    ) -> AppResult<ServiceMonitorView> {
        crate::service_monitor::validate_input(input)?;
        let existing = self.get_service_monitor(id)?;
        if let Some(server_id) = clean(input.server_id.as_deref()) {
            self.get_server(&server_id)?;
        }
        let changed = self.connection()?.execute(
            "UPDATE service_monitors SET name = ?2, kind = ?3, target = ?4, port = ?5,
                    server_id = ?6, interval_seconds = ?7, timeout_seconds = ?8,
                    expected_status = ?9, keyword = ?10, failure_threshold = ?11,
                    certificate_warning_days = ?12, enabled = ?13, updated_at = ?14
             WHERE id = ?1",
            params![
                id,
                input.name.trim(),
                service_monitor_kind(input.kind),
                input.target.trim(),
                input.port.filter(|port| *port != 0),
                clean(input.server_id.as_deref()),
                input.interval_seconds,
                input.timeout_seconds,
                input.expected_status,
                clean(input.keyword.as_deref()),
                input.failure_threshold,
                input.certificate_warning_days,
                input.enabled,
                Utc::now().to_rfc3339(),
            ],
        )?;
        if changed == 0 {
            return Err(AppError::Message("未找到这个服务监控".into()));
        }
        if existing.kind != input.kind || existing.target != input.target.trim() {
            let monitor = self.get_service_monitor(id)?;
            let connection = self.connection()?;
            for kind in [
                ServiceEventKind::Availability,
                ServiceEventKind::Certificate,
            ] {
                service_event_transition(
                    &connection,
                    &monitor,
                    kind,
                    false,
                    true,
                    "监控目标已变更，旧事件已关闭".into(),
                )?;
            }
        }
        self.service_monitor_view(id)
    }

    pub fn set_service_monitor_enabled(
        &self,
        id: &str,
        enabled: bool,
    ) -> AppResult<ServiceMonitorView> {
        let changed = self.connection()?.execute(
            "UPDATE service_monitors SET enabled = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, enabled, Utc::now().to_rfc3339()],
        )?;
        if changed == 0 {
            return Err(AppError::Message("未找到这个服务监控".into()));
        }
        self.service_monitor_view(id)
    }

    pub fn delete_service_monitor(&self, id: &str) -> AppResult<()> {
        self.connection()?
            .execute("DELETE FROM service_monitors WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn service_monitor_view(&self, id: &str) -> AppResult<ServiceMonitorView> {
        let monitor = self.get_service_monitor(id)?;
        let connection = self.connection()?;
        service_monitor_view(&connection, &monitor).map_err(Into::into)
    }

    pub fn list_service_samples(
        &self,
        monitor_id: &str,
        hours: u32,
    ) -> AppResult<Vec<ServiceSample>> {
        if !(1..=720).contains(&hours) {
            return Err(AppError::Message(
                "服务历史范围必须在 1 小时到 30 天之间".into(),
            ));
        }
        self.get_service_monitor(monitor_id)?;
        let cutoff = (Utc::now() - Duration::hours(i64::from(hours))).to_rfc3339();
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, monitor_id, success, latency_ms, status_code, detail,
                    certificate_expires_at, certificate_days_remaining, checked_at
             FROM service_samples WHERE monitor_id = ?1 AND checked_at >= ?2
             ORDER BY checked_at, id",
        )?;
        let samples = statement
            .query_map(params![monitor_id, cutoff], row_to_service_sample)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(downsample_service_samples(samples, 720))
    }

    pub fn list_service_events(&self, monitor_id: Option<&str>) -> AppResult<Vec<ServiceEvent>> {
        let connection = self.connection()?;
        let sql = "SELECT e.id, e.monitor_id, m.name, e.kind, e.status, e.message, e.created_at
                   FROM service_events e JOIN service_monitors m ON m.id = e.monitor_id";
        let mut events = Vec::new();
        if let Some(id) = monitor_id {
            let mut statement = connection.prepare(&format!(
                "{sql} WHERE e.monitor_id = ?1 ORDER BY e.id DESC LIMIT 100"
            ))?;
            let rows = statement.query_map([id], row_to_service_event)?;
            for event in rows {
                events.push(event?);
            }
        } else {
            let mut statement =
                connection.prepare(&format!("{sql} ORDER BY e.id DESC LIMIT 100"))?;
            let rows = statement.query_map([], row_to_service_event)?;
            for event in rows {
                events.push(event?);
            }
        }
        Ok(events)
    }

    pub fn record_service_probe(
        &self,
        monitor: &ServiceMonitor,
        probe: &ServiceProbe,
    ) -> AppResult<(ServiceMonitorView, Vec<ServiceEvent>)> {
        let checked_at = Utc::now().to_rfc3339();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO service_samples (
                monitor_id, success, latency_ms, status_code, detail,
                certificate_expires_at, certificate_days_remaining, checked_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                monitor.id,
                probe.success,
                probe.latency_ms,
                probe.status_code,
                probe.detail,
                probe.certificate_expires_at,
                probe.certificate_days_remaining,
                checked_at,
            ],
        )?;
        if self.should_prune() {
            let cutoff = (Utc::now() - Duration::days(30)).to_rfc3339();
            transaction.execute(
                "DELETE FROM service_samples WHERE checked_at < ?1",
                [cutoff],
            )?;
            transaction.execute(
                "DELETE FROM service_samples WHERE monitor_id = ?1 AND id IN (
                    SELECT id FROM service_samples WHERE monitor_id = ?1
                    ORDER BY id DESC LIMIT -1 OFFSET 50000
                 )",
                [&monitor.id],
            )?;
            let _ = transaction.execute(
                "DELETE FROM service_events WHERE id NOT IN (SELECT id FROM service_events ORDER BY id DESC LIMIT 1000)",
                [],
            );
        }
        let mut statement = transaction.prepare(
            "SELECT success FROM service_samples WHERE monitor_id = ?1 ORDER BY id DESC LIMIT 20",
        )?;
        let recent = statement
            .query_map([&monitor.id], |row| row.get::<_, bool>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        let consecutive_failures = recent.iter().take_while(|success| !**success).count() as u32;
        let mut events = Vec::new();
        let availability_message = if probe.success {
            format!("服务探测成功，已恢复。{}", probe.detail)
        } else {
            format!(
                "连续 {consecutive_failures} 次探测失败，达到故障阈值 {}。{}",
                monitor.failure_threshold, probe.detail
            )
        };
        if let Some(event) = service_event_transition(
            &transaction,
            monitor,
            ServiceEventKind::Availability,
            !probe.success && consecutive_failures >= monitor.failure_threshold,
            probe.success,
            availability_message,
        )? {
            events.push(event);
        }
        if let Some(days) = probe.certificate_days_remaining {
            let warning = monitor.certificate_warning_days as i32;
            let message = if days < 0 {
                format!("TLS 证书已过期 {} 天", days.saturating_abs())
            } else if days <= warning {
                format!("TLS 证书剩余 {days} 天，预警阈值为 {warning} 天")
            } else {
                format!("TLS 证书有效期已恢复至 {days} 天，高于预警阈值 {warning} 天")
            };
            if let Some(event) = service_event_transition(
                &transaction,
                monitor,
                ServiceEventKind::Certificate,
                days <= warning,
                days > warning,
                message,
            )? {
                events.push(event);
            }
        }
        transaction.commit()?;
        drop(connection);
        Ok((self.service_monitor_view(&monitor.id)?, events))
    }

    pub fn service_webhook_enabled(&self) -> AppResult<bool> {
        let value = self.connection()?.query_row(
            "SELECT value FROM app_settings WHERE key = 'service-webhook-enabled'",
            [],
            |row| row.get::<_, String>(0),
        );
        match value {
            Ok(value) => Ok(value == "true"),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    pub fn update_service_webhook_enabled(&self, enabled: bool) -> AppResult<()> {
        self.connection()?.execute(
            "INSERT INTO app_settings (key, value) VALUES ('service-webhook-enabled', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [if enabled { "true" } else { "false" }],
        )?;
        Ok(())
    }

    pub fn workspace_backup(&self) -> AppResult<WorkspaceBackup> {
        Ok(WorkspaceBackup {
            format_version: 1,
            created_at: Utc::now().to_rfc3339(),
            servers: self.list_servers()?,
            snippets: self.list_snippets()?,
            runbooks: self.list_runbooks()?,
            port_forwards: self.list_port_forwards()?,
            service_monitors: self
                .list_service_monitors()?
                .into_iter()
                .map(|view| view.monitor)
                .collect(),
            monitoring: self.monitoring_settings()?,
            service_webhook_enabled: Some(self.service_webhook_enabled()?),
            service_webhook_url: None,
            includes_credentials: false,
            credentials: std::collections::HashMap::new(),
            compose_environments: std::collections::HashMap::new(),
        })
    }

    pub fn insert_metric_sample(&self, metrics: &ServerMetrics) -> AppResult<()> {
        let memory_usage = usage_percent(metrics.memory_used, metrics.memory_total);
        let disk_usage = usage_percent(metrics.disk_used, metrics.disk_total);
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO metric_samples (
                server_id, cpu_usage, memory_usage, disk_usage, network_rx_bytes_per_second,
                network_tx_bytes_per_second, load_average, collected_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                metrics.server_id,
                metrics.cpu_usage,
                memory_usage,
                disk_usage,
                metrics.network_rx_bytes_per_second,
                metrics.network_tx_bytes_per_second,
                metrics.load_average[0],
                metrics.collected_at,
            ],
        )?;
        if self.should_prune() {
            let cutoff = (Utc::now() - Duration::days(7)).to_rfc3339();
            connection.execute(
                "DELETE FROM metric_samples WHERE collected_at < ?1",
                [cutoff],
            )?;
            connection.execute(
                "DELETE FROM metric_samples WHERE server_id = ?1 AND id IN (
                    SELECT id FROM metric_samples WHERE server_id = ?1
                    ORDER BY id DESC LIMIT -1 OFFSET 50000
                 )",
                [&metrics.server_id],
            )?;
            let _ = connection.execute(
                "DELETE FROM operation_logs WHERE id NOT IN (SELECT id FROM operation_logs ORDER BY id DESC LIMIT 2000)",
                [],
            );
            let _ = connection.execute(
                "DELETE FROM alert_events WHERE id NOT IN (SELECT id FROM alert_events ORDER BY id DESC LIMIT 1000)",
                [],
            );
            let _ = connection.execute(
                "DELETE FROM mcp_calls WHERE id NOT IN (SELECT id FROM mcp_calls ORDER BY id DESC LIMIT 1000)",
                [],
            );
            let _ = connection.execute(
                "DELETE FROM runbook_executions WHERE id NOT IN (SELECT id FROM runbook_executions ORDER BY created_at DESC LIMIT 500)",
                [],
            );
            let _ = connection.execute(
                "DELETE FROM data_protection_runs WHERE id NOT IN (SELECT id FROM data_protection_runs ORDER BY started_at DESC LIMIT 500)",
                [],
            );
            let _ = connection.execute(
                "DELETE FROM notification_deliveries WHERE id NOT IN (SELECT id FROM notification_deliveries ORDER BY id DESC LIMIT 500)",
                [],
            );
            let _ = connection.execute(
                "DELETE FROM server_benchmarks WHERE id NOT IN (SELECT id FROM server_benchmarks ORDER BY created_at DESC LIMIT 300)",
                [],
            );
        }
        Ok(())
    }

    pub fn list_metric_history(&self, server_id: &str, hours: u32) -> AppResult<Vec<MetricSample>> {
        if !(1..=168).contains(&hours) {
            return Err(AppError::Message(
                "历史趋势范围必须在 1 小时到 7 天之间".into(),
            ));
        }
        self.get_server(server_id)?;
        let cutoff = (Utc::now() - Duration::hours(i64::from(hours))).to_rfc3339();
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, server_id, cpu_usage, memory_usage, disk_usage,
                    network_rx_bytes_per_second, network_tx_bytes_per_second, load_average, collected_at
             FROM metric_samples WHERE server_id = ?1 AND collected_at >= ?2
             ORDER BY collected_at, id",
        )?;
        let rows = statement.query_map(params![server_id, cutoff], |row| {
            Ok(MetricSample {
                id: row.get(0)?,
                server_id: row.get(1)?,
                cpu_usage: row.get(2)?,
                memory_usage: row.get(3)?,
                disk_usage: row.get(4)?,
                network_rx_bytes_per_second: row.get(5)?,
                network_tx_bytes_per_second: row.get(6)?,
                load_average: row.get(7)?,
                collected_at: row.get(8)?,
            })
        })?;
        let samples = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(downsample(samples, 720))
    }

    pub fn monitoring_settings(&self) -> AppResult<MonitoringSettings> {
        let connection = self.connection()?;
        let value = connection.query_row(
            "SELECT value FROM app_settings WHERE key = 'monitoring'",
            [],
            |row| row.get::<_, String>(0),
        );
        match value {
            Ok(value) => serde_json::from_str(&value)
                .map_err(|error| AppError::Message(format!("监控设置无法读取：{error}"))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(MonitoringSettings::default()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn update_monitoring_settings(
        &self,
        settings: &MonitoringSettings,
    ) -> AppResult<MonitoringSettings> {
        validate_monitoring_settings(settings)?;
        let value = serde_json::to_string(settings)
            .map_err(|error| AppError::Message(error.to_string()))?;
        self.connection()?.execute(
            "INSERT INTO app_settings (key, value) VALUES ('monitoring', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [value],
        )?;
        Ok(settings.clone())
    }

    pub fn mcp_settings(&self) -> AppResult<McpSettings> {
        let stored = self
            .connection()?
            .query_row(
                "SELECT value FROM app_settings WHERE key = 'mcp'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let mut settings = stored
            .as_deref()
            .and_then(|value| serde_json::from_str::<McpSettings>(value).ok())
            .unwrap_or_default();
        // Prefer OS Keychain for the MCP bearer token; migrate any legacy plaintext SQLite token once.
        let legacy_token = std::mem::take(&mut settings.token);
        let token = match load_mcp_token() {
            Ok(Some(secret)) if !secret.is_empty() => {
                if !legacy_token.is_empty() {
                    let _ = self.store_mcp_settings(&settings);
                }
                secret
            }
            Ok(_) => {
                let next = if !legacy_token.is_empty() {
                    legacy_token
                } else {
                    random_token()
                };
                if store_mcp_token(&next).is_ok() {
                    let _ = self.store_mcp_settings(&settings);
                }
                next
            }
            Err(_) => {
                if !legacy_token.is_empty() {
                    legacy_token
                } else {
                    random_token()
                }
            }
        };
        settings.token = token;
        Ok(settings)
    }

    pub fn update_mcp_settings(&self, input: &McpSettingsInput) -> AppResult<McpSettings> {
        if !(1024..=65535).contains(&input.port) {
            return Err(AppError::Message(
                "MCP 端口必须在 1024 到 65535 之间".into(),
            ));
        }
        let current = self.mcp_settings()?;
        let write_policy = input.write_policy.unwrap_or(if input.allow_execute {
            McpWritePolicy::Allow
        } else {
            McpWritePolicy::ReadOnly
        });
        let mut settings = McpSettings {
            enabled: input.enabled,
            port: input.port,
            allow_execute: write_policy != McpWritePolicy::ReadOnly,
            write_policy: Some(write_policy),
            allow_unknown_host: input.allow_unknown_host,
            token: String::new(),
        };
        self.store_mcp_settings(&settings)?;
        settings.token = current.token;
        Ok(settings)
    }

    pub fn ai_settings(&self) -> AppResult<AiSettings> {
        let raw = self.get_app_setting("ai")?;
        let settings = match raw {
            Some(value) => serde_json::from_str::<AiSettings>(&value).unwrap_or_default(),
            None => AiSettings::default(),
        };
        Ok(settings)
    }

    pub fn save_ai_settings(&self, settings: &AiSettings) -> AppResult<()> {
        let value = serde_json::to_string(settings)
            .map_err(|error| AppError::Message(error.to_string()))?;
        self.set_app_setting("ai", &value)
    }

    pub fn save_server_benchmark(&self, benchmark: &ServerBenchmark) -> AppResult<()> {
        let payload = serde_json::to_string(benchmark)
            .map_err(|error| AppError::Message(error.to_string()))?;
        self.connection()?.execute(
            "INSERT INTO server_benchmarks (id, server_id, payload, score, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                benchmark.id,
                benchmark.server_id,
                payload,
                benchmark.score,
                benchmark.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn list_server_benchmarks(
        &self,
        server_id: Option<&str>,
    ) -> AppResult<Vec<ServerBenchmark>> {
        let connection = self.connection()?;
        let mut items = Vec::new();
        if let Some(id) = server_id {
            let mut stmt = connection.prepare(
                "SELECT payload FROM server_benchmarks WHERE server_id = ?1
                 ORDER BY created_at DESC LIMIT 20",
            )?;
            let rows = stmt.query_map([id], |row| row.get::<_, String>(0))?;
            for row in rows {
                if let Ok(parsed) = serde_json::from_str::<ServerBenchmark>(&row?) {
                    items.push(parsed);
                }
            }
        } else {
            let mut stmt = connection.prepare(
                "SELECT payload FROM server_benchmarks
                 WHERE id IN (
                    SELECT id FROM server_benchmarks b2
                    WHERE b2.server_id = server_benchmarks.server_id
                    ORDER BY created_at DESC LIMIT 1
                 )
                 ORDER BY score DESC, created_at DESC LIMIT 100",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            for row in rows {
                if let Ok(parsed) = serde_json::from_str::<ServerBenchmark>(&row?) {
                    items.push(parsed);
                }
            }
        }
        Ok(items)
    }

    pub fn rotate_mcp_token(&self) -> AppResult<McpSettings> {
        let mut settings = self.mcp_settings()?;
        let next_token = random_token();
        store_mcp_token(&next_token)?;
        settings.token = String::new();
        self.store_mcp_settings(&settings)?;
        settings.token = next_token;
        Ok(settings)
    }

    fn store_mcp_settings(&self, settings: &McpSettings) -> AppResult<()> {
        // Never persist the plaintext MCP bearer token into SQLite app_settings.
        let redacted = McpSettings {
            token: String::new(),
            ..settings.clone()
        };
        let value = serde_json::to_string(&redacted)
            .map_err(|error| AppError::Message(error.to_string()))?;
        self.connection()?.execute(
            "INSERT INTO app_settings (key, value) VALUES ('mcp', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [value],
        )?;
        Ok(())
    }

    pub fn record_mcp_call(
        &self,
        tool: &str,
        target: &str,
        success: bool,
        duration_ms: u64,
        summary: &str,
    ) -> AppResult<McpCallRecord> {
        let created_at = Utc::now().to_rfc3339();
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO mcp_calls (tool, target, success, duration_ms, summary, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                tool,
                target,
                success,
                duration_ms as i64,
                truncate_chars(summary, 500),
                created_at
            ],
        )?;
        let id = connection.last_insert_rowid();
        Ok(McpCallRecord {
            id,
            tool: tool.to_string(),
            target: target.to_string(),
            success,
            duration_ms,
            summary: truncate_chars(summary, 500),
            created_at,
        })
    }

    pub fn list_mcp_calls(&self, limit: usize) -> AppResult<Vec<McpCallRecord>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, tool, target, success, duration_ms, summary, created_at
             FROM mcp_calls ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([limit.clamp(1, 500) as i64], |row| {
            Ok(McpCallRecord {
                id: row.get(0)?,
                tool: row.get(1)?,
                target: row.get(2)?,
                success: row.get(3)?,
                duration_ms: row.get(4)?,
                summary: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?;
        let mut calls = rows.collect::<Result<Vec<_>, _>>()?;
        calls.sort_by_key(|call| std::cmp::Reverse(call.id));
        Ok(calls)
    }

    pub fn clear_mcp_calls(&self) -> AppResult<()> {
        self.connection()?.execute("DELETE FROM mcp_calls", [])?;
        Ok(())
    }

    pub fn record_alert(
        &self,
        server: &Server,
        kind: AlertKind,
        status: AlertStatus,
        message: String,
        value: Option<f64>,
        threshold: Option<f64>,
    ) -> AppResult<AlertEvent> {
        let created_at = Utc::now().to_rfc3339();
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO alert_events (
                server_id, kind, status, message, value, threshold, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                server.id,
                alert_kind(kind),
                alert_status(status),
                message,
                value,
                threshold,
                created_at,
            ],
        )?;
        Ok(AlertEvent {
            id: connection.last_insert_rowid(),
            server_id: server.id.clone(),
            server_name: server.name.clone(),
            kind,
            status,
            message,
            value,
            threshold,
            created_at,
        })
    }

    pub fn list_alert_events(&self, server_id: Option<&str>) -> AppResult<Vec<AlertEvent>> {
        let connection = self.connection()?;
        let sql = "SELECT a.id, a.server_id, s.name, a.kind, a.status, a.message,
                          a.value, a.threshold, a.created_at
                   FROM alert_events a JOIN servers s ON s.id = a.server_id";
        let mut events = Vec::new();
        if let Some(id) = server_id {
            let mut statement = connection.prepare(&format!(
                "{sql} WHERE a.server_id = ?1 ORDER BY a.id DESC LIMIT 100"
            ))?;
            let rows = statement.query_map([id], row_to_alert_event)?;
            for row in rows {
                events.push(row?);
            }
        } else {
            let mut statement =
                connection.prepare(&format!("{sql} ORDER BY a.id DESC LIMIT 100"))?;
            let rows = statement.query_map([], row_to_alert_event)?;
            for row in rows {
                events.push(row?);
            }
        }
        Ok(events)
    }

    pub fn list_active_alerts(&self) -> AppResult<Vec<AlertEvent>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT a.id, a.server_id, s.name, a.kind, a.status, a.message,
                    a.value, a.threshold, a.created_at
             FROM alert_events a JOIN servers s ON s.id = a.server_id
             WHERE a.status = 'triggered' AND a.id IN (
                SELECT MAX(id) FROM alert_events GROUP BY server_id, kind
             ) ORDER BY a.id DESC",
        )?;
        let rows = statement.query_map([], row_to_alert_event)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_app_setting(&self, key: &str) -> AppResult<Option<String>> {
        self.connection()?
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn set_app_setting(&self, key: &str, value: &str) -> AppResult<()> {
        self.connection()?.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn list_notification_channels(&self) -> AppResult<Vec<NotificationChannel>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, name, kind, enabled, target, events, min_severity, quiet_start,
                    quiet_end, has_secret, has_signing_secret, created_at, updated_at
             FROM notification_channels ORDER BY created_at DESC",
        )?;
        let rows = statement.query_map([], row_to_notification_channel)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_notification_channel(&self, id: &str) -> AppResult<NotificationChannel> {
        self.connection()?
            .query_row(
                "SELECT id, name, kind, enabled, target, events, min_severity, quiet_start,
                        quiet_end, has_secret, has_signing_secret, created_at, updated_at
                 FROM notification_channels WHERE id = ?1",
                [id],
                row_to_notification_channel,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    AppError::Message("未找到这个通知渠道".into())
                }
                other => other.into(),
            })
    }

    pub fn create_notification_channel(
        &self,
        id: &str,
        input: &NotificationChannelInput,
    ) -> AppResult<NotificationChannel> {
        let now = Utc::now().to_rfc3339();
        let has_secret = input
            .secret
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        let has_signing_secret = input
            .signing_secret
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        self.connection()?.execute(
            "INSERT INTO notification_channels (
                id, name, kind, enabled, target, events, min_severity, quiet_start, quiet_end,
                has_secret, has_signing_secret, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                id,
                input.name.trim(),
                notification_channel_kind(input.kind),
                input.enabled,
                input.target.trim(),
                json_string(&input.events)?,
                notification_severity(input.min_severity),
                clean(input.quiet_start.as_deref()),
                clean(input.quiet_end.as_deref()),
                has_secret,
                has_signing_secret,
                now,
                now,
            ],
        )?;
        self.get_notification_channel(id)
    }

    pub fn update_notification_channel(
        &self,
        id: &str,
        input: &NotificationChannelInput,
    ) -> AppResult<NotificationChannel> {
        let existing = self.get_notification_channel(id)?;
        let has_secret = existing.has_secret
            || input
                .secret
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty());
        let has_signing_secret = if input.clear_signing_secret {
            false
        } else {
            existing.has_signing_secret
                || input
                    .signing_secret
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
        };
        self.connection()?.execute(
            "UPDATE notification_channels
             SET name = ?2, kind = ?3, enabled = ?4, target = ?5, events = ?6,
                 min_severity = ?7, quiet_start = ?8, quiet_end = ?9,
                 has_secret = ?10, has_signing_secret = ?11, updated_at = ?12
             WHERE id = ?1",
            params![
                id,
                input.name.trim(),
                notification_channel_kind(input.kind),
                input.enabled,
                input.target.trim(),
                json_string(&input.events)?,
                notification_severity(input.min_severity),
                clean(input.quiet_start.as_deref()),
                clean(input.quiet_end.as_deref()),
                has_secret,
                has_signing_secret,
                Utc::now().to_rfc3339(),
            ],
        )?;
        self.get_notification_channel(id)
    }

    pub fn set_notification_channel_enabled(
        &self,
        id: &str,
        enabled: bool,
    ) -> AppResult<NotificationChannel> {
        let changed = self.connection()?.execute(
            "UPDATE notification_channels SET enabled = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, enabled, Utc::now().to_rfc3339()],
        )?;
        if changed == 0 {
            return Err(AppError::Message("未找到这个通知渠道".into()));
        }
        self.get_notification_channel(id)
    }

    pub fn delete_notification_channel(&self, id: &str) -> AppResult<()> {
        self.connection()?
            .execute("DELETE FROM notification_channels WHERE id = ?1", [id])?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_notification_delivery(
        &self,
        channel_id: &str,
        channel_name: &str,
        event_kind: NotificationEventKind,
        severity: NotificationSeverity,
        title: &str,
        message: &str,
        success: bool,
        error: Option<&str>,
        attempts: u32,
    ) -> AppResult<NotificationDelivery> {
        let now = Utc::now().to_rfc3339();
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO notification_deliveries (
                channel_id, channel_name, event_kind, severity, title, message,
                success, error, attempts, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                channel_id,
                channel_name,
                notification_event_kind(event_kind),
                notification_severity(severity),
                title,
                message,
                success,
                error,
                attempts,
                now,
            ],
        )?;
        let id = connection.last_insert_rowid();
        Ok(NotificationDelivery {
            id,
            channel_id: channel_id.to_string(),
            channel_name: channel_name.to_string(),
            event_kind,
            severity,
            title: title.to_string(),
            message: message.to_string(),
            success,
            error: error.map(str::to_string),
            attempts,
            created_at: now,
        })
    }

    pub fn list_notification_deliveries(
        &self,
        limit: usize,
    ) -> AppResult<Vec<NotificationDelivery>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, channel_id, channel_name, event_kind, severity, title, message,
                    success, error, attempts, created_at
             FROM notification_deliveries ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([limit.clamp(1, 200) as i64], row_to_notification_delivery)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn clear_notification_deliveries(&self) -> AppResult<()> {
        self.connection()?
            .execute("DELETE FROM notification_deliveries", [])?;
        Ok(())
    }

    pub fn action_center_summary(&self) -> AppResult<ActionCenterSummary> {
        let servers = self.list_servers()?;
        let today = Utc::now().date_naive();
        let mut expiring_servers = Vec::new();
        for server in &servers {
            if let Some(raw) = server.expires_at.as_deref() {
                if let Some(date) = parse_expiry_date(raw) {
                    let days = (date - today).num_days();
                    if days <= 30 {
                        expiring_servers.push(ExpiringServerItem {
                            server_id: server.id.clone(),
                            server_name: server.name.clone(),
                            provider: server.provider.clone(),
                            expires_at: raw.to_string(),
                            days_remaining: days,
                        });
                    }
                }
            }
        }
        expiring_servers.sort_by_key(|item| item.days_remaining);

        let active_host_alerts = self.list_active_alerts()?;
        let connection = self.connection()?;

        let mut stmt = connection.prepare(
            "SELECT e.id, e.monitor_id, m.name, e.kind, e.status, e.message, e.created_at
             FROM service_events e JOIN service_monitors m ON m.id = e.monitor_id
             WHERE e.status = 'triggered' AND e.id IN (
                SELECT MAX(id) FROM service_events GROUP BY monitor_id, kind
             ) ORDER BY e.id DESC LIMIT 20",
        )?;
        let active_service_events = stmt
            .query_map([], row_to_service_event)?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);

        let mut stmt = connection.prepare(
            "SELECT m.id, m.name, m.target, s.certificate_expires_at,
                    s.certificate_days_remaining, m.certificate_warning_days
             FROM service_monitors m
             JOIN service_samples s ON s.id = (
                SELECT MAX(id) FROM service_samples WHERE monitor_id = m.id
             )
             WHERE m.enabled = 1
               AND s.certificate_expires_at IS NOT NULL
               AND s.certificate_days_remaining IS NOT NULL
               AND s.certificate_days_remaining <= m.certificate_warning_days
             ORDER BY s.certificate_days_remaining ASC",
        )?;
        let expiring_certificates = stmt
            .query_map([], |row| {
                Ok(ExpiringCertificateItem {
                    monitor_id: row.get(0)?,
                    monitor_name: row.get(1)?,
                    target: row.get(2)?,
                    expires_at: row.get(3)?,
                    days_remaining: row.get(4)?,
                    warning_days: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);

        let mut stmt = connection.prepare(
            "SELECT p.id, p.name, p.server_id, s.name, COALESCE(r.error, p.last_error, '备份执行失败'),
                    COALESCE(r.finished_at, r.started_at, p.updated_at)
             FROM data_protection_policies p
             JOIN servers s ON s.id = p.server_id
             JOIN data_protection_runs r ON r.id = (
                SELECT id FROM data_protection_runs
                WHERE policy_id = p.id ORDER BY started_at DESC LIMIT 1
             )
             WHERE r.status = 'failed'
             ORDER BY r.started_at DESC LIMIT 20",
        )?;
        let failed_backups = stmt
            .query_map([], |row| {
                Ok(FailedBackupItem {
                    policy_id: row.get(0)?,
                    policy_name: row.get(1)?,
                    server_id: row.get(2)?,
                    server_name: row.get(3)?,
                    error: row.get(4)?,
                    failed_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);

        let cutoff = (Utc::now() - Duration::hours(72)).to_rfc3339();
        let mut stmt = connection.prepare(
            "SELECT id, runbook_name, status, failure_count, error, created_at
             FROM runbook_executions
             WHERE created_at >= ?1
               AND status IN ('failed', 'partial', 'timed_out', 'interrupted')
             ORDER BY created_at DESC LIMIT 10",
        )?;
        let failed_runbooks = stmt
            .query_map([cutoff], |row| {
                Ok(FailedRunbookItem {
                    execution_id: row.get(0)?,
                    runbook_name: row.get(1)?,
                    status: parse_enum(&row.get::<_, String>(2)?)?,
                    failure_count: row.get(3)?,
                    error: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);

        let enabled_notification_channels: i64 = connection.query_row(
            "SELECT COUNT(*) FROM notification_channels WHERE enabled = 1",
            [],
            |row| row.get(0),
        )?;
        let scheduled_backup_policies: i64 = connection.query_row(
            "SELECT COUNT(*) FROM data_protection_policies WHERE schedule_enabled = 1",
            [],
            |row| row.get(0),
        )?;

        Ok(ActionCenterSummary {
            expiring_servers,
            active_host_alerts,
            active_service_events,
            expiring_certificates,
            failed_backups,
            failed_runbooks,
            enabled_notification_channels: enabled_notification_channels.max(0) as usize,
            scheduled_backup_policies: scheduled_backup_policies.max(0) as usize,
        })
    }
}

fn service_monitor_view(
    connection: &Connection,
    monitor: &ServiceMonitor,
) -> rusqlite::Result<ServiceMonitorView> {
    let latest = connection
        .query_row(
            "SELECT id, monitor_id, success, latency_ms, status_code, detail,
                    certificate_expires_at, certificate_days_remaining, checked_at
             FROM service_samples WHERE monitor_id = ?1 ORDER BY id DESC LIMIT 1",
            [&monitor.id],
            row_to_service_sample,
        )
        .optional()?;
    let mut statement = connection.prepare(
        "SELECT e.id, e.monitor_id, m.name, e.kind, e.status, e.message, e.created_at
         FROM service_events e JOIN service_monitors m ON m.id = e.monitor_id
         WHERE e.monitor_id = ?1 AND e.status = 'triggered' AND e.id IN (
            SELECT MAX(id) FROM service_events GROUP BY monitor_id, kind
         ) ORDER BY e.id DESC",
    )?;
    let active_events = statement
        .query_map([&monitor.id], row_to_service_event)?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    let mut statement = connection.prepare(
        "SELECT success FROM service_samples WHERE monitor_id = ?1 ORDER BY id DESC LIMIT 20",
    )?;
    let recent = statement
        .query_map([&monitor.id], |row| row.get::<_, bool>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    let consecutive_failures = recent.iter().take_while(|success| !**success).count() as u32;
    let (uptime_24h, uptime_7d, uptime_30d) = service_uptimes(connection, &monitor.id)?;
    Ok(ServiceMonitorView {
        monitor: monitor.clone(),
        latest,
        active_events,
        consecutive_failures,
        uptime_24h,
        uptime_7d,
        uptime_30d,
    })
}

fn service_uptimes(
    connection: &Connection,
    monitor_id: &str,
) -> rusqlite::Result<(Option<f64>, Option<f64>, Option<f64>)> {
    let now = Utc::now();
    let cutoff_24h = (now - Duration::hours(24)).to_rfc3339();
    let cutoff_7d = (now - Duration::days(7)).to_rfc3339();
    let cutoff_30d = (now - Duration::days(30)).to_rfc3339();
    let row: (i64, i64, i64, i64, i64, i64) = connection.query_row(
        "SELECT
            COALESCE(SUM(CASE WHEN checked_at >= ?2 THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN checked_at >= ?2 AND success = 1 THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN checked_at >= ?3 THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN checked_at >= ?3 AND success = 1 THEN 1 ELSE 0 END), 0),
            COUNT(*),
            COALESCE(SUM(CASE WHEN success = 1 THEN 1 ELSE 0 END), 0)
         FROM service_samples
         WHERE monitor_id = ?1 AND checked_at >= ?4",
        params![monitor_id, cutoff_24h, cutoff_7d, cutoff_30d],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        },
    )?;
    let pct = |total: i64, ok: i64| (total > 0).then(|| ok as f64 / total as f64 * 100.0);
    Ok((pct(row.0, row.1), pct(row.2, row.3), pct(row.4, row.5)))
}

fn load_mcp_token() -> AppResult<Option<String>> {
    #[cfg(test)]
    {
        Ok(None)
    }
    #[cfg(not(test))]
    {
        utils::get_secret(MCP_TOKEN_ACCOUNT)
    }
}

fn store_mcp_token(token: &str) -> AppResult<()> {
    #[cfg(test)]
    {
        let _ = token;
        Ok(())
    }
    #[cfg(not(test))]
    {
        utils::set_secret(MCP_TOKEN_ACCOUNT, token)
    }
}

fn service_event_transition(
    connection: &Connection,
    monitor: &ServiceMonitor,
    kind: ServiceEventKind,
    triggered: bool,
    recovered: bool,
    message: String,
) -> rusqlite::Result<Option<ServiceEvent>> {
    let previous = connection
        .query_row(
            "SELECT status FROM service_events WHERE monitor_id = ?1 AND kind = ?2 ORDER BY id DESC LIMIT 1",
            params![monitor.id, service_event_kind(kind)],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let active = previous.as_deref() == Some("triggered");
    let status = if triggered && !active {
        AlertStatus::Triggered
    } else if recovered && active {
        AlertStatus::Recovered
    } else {
        return Ok(None);
    };
    let created_at = Utc::now().to_rfc3339();
    connection.execute(
        "INSERT INTO service_events (monitor_id, kind, status, message, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            monitor.id,
            service_event_kind(kind),
            alert_status(status),
            message,
            created_at,
        ],
    )?;
    Ok(Some(ServiceEvent {
        id: connection.last_insert_rowid(),
        monitor_id: monitor.id.clone(),
        monitor_name: monitor.name.clone(),
        kind,
        status,
        message,
        created_at,
    }))
}

fn usage_percent(used: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        (used as f64 / total as f64 * 100.0).clamp(0.0, 100.0)
    }
}

fn downsample(samples: Vec<MetricSample>, limit: usize) -> Vec<MetricSample> {
    if samples.len() <= limit {
        return samples;
    }
    let last = samples.len() - 1;
    (0..limit)
        .map(|index| samples[index * last / (limit - 1)].clone())
        .collect()
}

fn downsample_service_samples(samples: Vec<ServiceSample>, limit: usize) -> Vec<ServiceSample> {
    if samples.len() <= limit {
        return samples;
    }
    let last = samples.len() - 1;
    (0..limit)
        .map(|index| samples[index * last / (limit - 1)].clone())
        .collect()
}

fn validate_monitoring_settings(settings: &MonitoringSettings) -> AppResult<()> {
    if !(15..=3600).contains(&settings.interval_seconds) {
        return Err(AppError::Message(
            "采集间隔必须在 15 秒到 1 小时之间".into(),
        ));
    }
    if [
        settings.cpu_threshold,
        settings.memory_threshold,
        settings.disk_threshold,
    ]
    .into_iter()
    .any(|value| !value.is_finite() || !(1.0..=100.0).contains(&value))
    {
        return Err(AppError::Message("告警阈值必须在 1% 到 100% 之间".into()));
    }
    if !(1..=20).contains(&settings.offline_after) {
        return Err(AppError::Message("离线判定次数必须在 1 到 20 之间".into()));
    }
    if !(1..=1440).contains(&settings.cooldown_minutes) {
        return Err(AppError::Message(
            "告警冷却时间必须在 1 分钟到 24 小时之间".into(),
        ));
    }
    Ok(())
}

fn service_monitor_kind(kind: ServiceMonitorKind) -> &'static str {
    match kind {
        ServiceMonitorKind::Http => "http",
        ServiceMonitorKind::Tcp => "tcp",
        ServiceMonitorKind::Dns => "dns",
    }
}

fn service_event_kind(kind: ServiceEventKind) -> &'static str {
    match kind {
        ServiceEventKind::Availability => "availability",
        ServiceEventKind::Certificate => "certificate",
    }
}

fn alert_kind(kind: AlertKind) -> &'static str {
    match kind {
        AlertKind::Offline => "offline",
        AlertKind::Cpu => "cpu",
        AlertKind::Memory => "memory",
        AlertKind::Disk => "disk",
    }
}

fn alert_status(status: AlertStatus) -> &'static str {
    match status {
        AlertStatus::Triggered => "triggered",
        AlertStatus::Recovered => "recovered",
    }
}

fn validate_data_protection(
    input: &DataProtectionInput,
    has_existing_password: bool,
    has_existing_s3: bool,
) -> AppResult<()> {
    if input.name.trim().is_empty() || input.name.chars().count() > 120 {
        return Err(AppError::Message(
            "策略名称不能为空且不能超过 120 个字符".into(),
        ));
    }
    if !(1..=100).contains(&input.keep_last) {
        return Err(AppError::Message("保留快照数量必须在 1 到 100 之间".into()));
    }
    let repository = input.repository.trim();
    match input.repository_kind {
        ResticRepositoryKind::Local
            if !repository.starts_with('/') || repository.contains('\n') =>
        {
            return Err(AppError::Message("本地 Restic 仓库必须是绝对路径".into()));
        }
        ResticRepositoryKind::S3
            if !(repository.starts_with("s3:https://") || repository.starts_with("s3:http://"))
                || repository.contains('\n') =>
        {
            return Err(AppError::Message(
                "S3 仓库格式为 s3:https://endpoint/bucket/可选前缀".into(),
            ));
        }
        _ => {}
    }
    if repository.len() > 2048 {
        return Err(AppError::Message("Restic 仓库地址过长".into()));
    }
    let has_password = input
        .repository_password
        .as_deref()
        .is_some_and(|value| !value.is_empty())
        || has_existing_password;
    if !has_password {
        return Err(AppError::Message("请设置 Restic 仓库密码".into()));
    }
    if input.repository_kind == ResticRepositoryKind::S3 {
        let has_s3 = (input
            .s3_access_key_id
            .as_deref()
            .is_some_and(|value| !value.is_empty())
            && input
                .s3_secret_access_key
                .as_deref()
                .is_some_and(|value| !value.is_empty()))
            || has_existing_s3;
        if !has_s3 {
            return Err(AppError::Message(
                "S3 仓库需要 Access Key 和 Secret Key".into(),
            ));
        }
    }
    match input.source_kind {
        ProtectionSourceKind::Directory => {
            if input.paths.is_empty()
                || input.paths.len() > 20
                || input
                    .paths
                    .iter()
                    .any(|path| !path.starts_with('/') || path.contains('\n') || path.len() > 4096)
            {
                return Err(AppError::Message(
                    "目录备份需要 1 到 20 个远程绝对路径".into(),
                ));
            }
        }
        ProtectionSourceKind::DockerVolume => {
            let volume = input.volume_name.as_deref().unwrap_or("");
            if volume.is_empty()
                || !volume.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-')
                })
            {
                return Err(AppError::Message("Docker 卷名称包含无效字符".into()));
            }
        }
        ProtectionSourceKind::Postgresql | ProtectionSourceKind::Mysql => {
            if clean(input.database_container.as_deref()).is_none()
                || clean(input.database_name.as_deref()).is_none()
            {
                return Err(AppError::Message(
                    "数据库备份需要容器名称和数据库名称".into(),
                ));
            }
        }
    }
    Ok(())
}

fn restic_repository_kind(kind: ResticRepositoryKind) -> &'static str {
    match kind {
        ResticRepositoryKind::Local => "local",
        ResticRepositoryKind::S3 => "s3",
    }
}

fn protection_source_kind(kind: ProtectionSourceKind) -> &'static str {
    match kind {
        ProtectionSourceKind::Directory => "directory",
        ProtectionSourceKind::DockerVolume => "docker_volume",
        ProtectionSourceKind::Postgresql => "postgresql",
        ProtectionSourceKind::Mysql => "mysql",
    }
}

fn data_protection_action(action: DataProtectionAction) -> &'static str {
    match action {
        DataProtectionAction::Initialize => "initialize",
        DataProtectionAction::Check => "check",
        DataProtectionAction::Backup => "backup",
        DataProtectionAction::Restore => "restore",
    }
}

fn data_protection_run_status(status: DataProtectionRunStatus) -> &'static str {
    match status {
        DataProtectionRunStatus::Running => "running",
        DataProtectionRunStatus::Succeeded => "succeeded",
        DataProtectionRunStatus::Failed => "failed",
    }
}

fn validate_port_forward(input: &PortForwardInput) -> AppResult<()> {
    if input.name.trim().is_empty() {
        return Err(AppError::Message("规则名称不能为空".into()));
    }
    if input.kind != crate::models::PortForwardKind::Dynamic
        && input.destination_host.trim().is_empty()
    {
        return Err(AppError::Message("固定转发的目标地址不能为空".into()));
    }
    if input.kind != crate::models::PortForwardKind::Dynamic && input.destination_port == 0 {
        return Err(AppError::Message(
            "目标端口必须在 1 到 65535 之间；监听端口可填 0 自动分配".into(),
        ));
    }
    input
        .listen_address
        .parse::<std::net::IpAddr>()
        .map_err(|_| AppError::Message("监听地址必须是有效的 IP 地址，例如 127.0.0.1".into()))?;
    Ok(())
}

fn normalized_steps(input: &RunbookInput) -> Vec<RunbookStep> {
    input
        .steps
        .iter()
        .map(|step| RunbookStep {
            id: Uuid::new_v4().to_string(),
            name: step.name.trim().to_string(),
            command: step.command.clone(),
        })
        .collect()
}

fn validate_runbook(input: &RunbookInput) -> AppResult<()> {
    if input.name.trim().is_empty() || input.name.chars().count() > 120 {
        return Err(AppError::Message(
            "Runbook 名称不能为空且不能超过 120 个字符".into(),
        ));
    }
    if input.steps.is_empty() || input.steps.len() > 30 {
        return Err(AppError::Message("每个 Runbook 需要 1 到 30 个步骤".into()));
    }
    if input.steps.iter().any(|step| {
        step.name.trim().is_empty()
            || step.name.chars().count() > 120
            || step.command.trim().is_empty()
            || step.command.len() > 100_000
    }) {
        return Err(AppError::Message(
            "每个步骤都需要名称和命令，名称不超过 120 个字符，命令不超过 100000 字节".into(),
        ));
    }
    Ok(())
}

fn validate_snippet(input: &SnippetInput) -> AppResult<()> {
    if input.name.trim().is_empty() || input.command.trim().is_empty() {
        return Err(AppError::Message("片段名称和命令不能为空".into()));
    }
    if input.name.chars().count() > 120 || input.command.len() > 100_000 {
        return Err(AppError::Message("片段名称或命令内容过长".into()));
    }
    Ok(())
}

fn validate_jump_route(
    connection: &Connection,
    server_id: &str,
    jump_server_id: Option<&str>,
    jump_chain: &[String],
) -> AppResult<()> {
    let mut hops: Vec<String> = jump_chain
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    if hops.is_empty() {
        if let Some(single) = clean(jump_server_id) {
            hops.push(single);
        }
    }
    if hops.len() > 4 {
        return Err(AppError::Message("跳板链最多支持 4 跳".into()));
    }
    let mut seen = HashSet::new();
    for hop in &hops {
        if hop == server_id {
            return Err(AppError::Message("服务器不能使用自身作为跳板机".into()));
        }
        if !seen.insert(hop.clone()) {
            return Err(AppError::Message(
                "跳板链中不能重复使用同一台服务器".into(),
            ));
        }
        let exists = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM servers WHERE id = ?1)",
            [hop],
            |row| row.get::<_, bool>(0),
        )?;
        if !exists {
            return Err(AppError::Message("所选跳板机不存在".into()));
        }
    }
    for hop in &hops {
        if route_reaches(connection, hop, server_id)? {
            return Err(AppError::Message(
                "跳板链形成环路，请调整连接路由".into(),
            ));
        }
    }
    Ok(())
}

fn effective_route(connection: &Connection, server_id: &str) -> AppResult<Vec<String>> {
    let row = connection.query_row(
        "SELECT s.jump_chain, s.jump_server_id, s.inherit_jump, g.jump_server_id
         FROM servers s LEFT JOIN server_groups g ON g.name = s.group_name
         WHERE s.id = ?1",
        [server_id],
        |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, bool>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        },
    );
    let (chain, jump, inherit, group_jump) = match row {
        Ok(value) => value,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let chain: Vec<String> = chain
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    if !chain.is_empty() {
        return Ok(chain);
    }
    if let Some(jump) = jump {
        return Ok(vec![jump]);
    }
    if inherit {
        if let Some(group_jump) = group_jump {
            return Ok(vec![group_jump]);
        }
    }
    Ok(Vec::new())
}

fn route_reaches(connection: &Connection, start: &str, target: &str) -> AppResult<bool> {
    let mut stack = vec![start.to_string()];
    let mut visited = HashSet::new();
    while let Some(current) = stack.pop() {
        if !visited.insert(current.clone()) {
            continue;
        }
        if current == target {
            return Ok(true);
        }
        if visited.len() > 12 {
            return Ok(false);
        }
        for next in effective_route(connection, &current)? {
            stack.push(next);
        }
    }
    Ok(false)
}

fn validate_server(input: &ServerInput) -> AppResult<()> {
    if input.name.trim().is_empty()
        || input.host.trim().is_empty()
        || input.username.trim().is_empty()
    {
        return Err(AppError::Message("名称、主机和用户名不能为空".into()));
    }
    if input.port == 0 {
        return Err(AppError::Message("SSH 端口必须在 1 到 65535 之间".into()));
    }
    Ok(())
}

fn random_token() -> String {
    let mut bytes = [0_u8; 24];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let truncated: String = value.chars().take(limit).collect();
    format!("{truncated}…")
}

fn clean(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn forward_kind(kind: crate::models::PortForwardKind) -> &'static str {
    match kind {
        crate::models::PortForwardKind::Local => "local",
        crate::models::PortForwardKind::Remote => "remote",
        crate::models::PortForwardKind::Dynamic => "dynamic",
    }
}

fn cloud_provider_name(provider: CloudProvider) -> &'static str {
    match provider {
        CloudProvider::DigitalOcean => "digital_ocean",
        CloudProvider::Tailscale => "tailscale",
    }
}

fn unique_group_error(error: rusqlite::Error) -> AppError {
    match &error {
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            AppError::Message("分组名称已存在".into())
        }
        _ => error.into(),
    }
}

fn validate_group(db: &Database, input: &ServerGroupInput) -> AppResult<()> {
    if input.name.trim().is_empty() || input.name.chars().count() > 80 {
        return Err(AppError::Message(
            "分组名称不能为空且不能超过 80 个字符".into(),
        ));
    }
    if let Some(port) = input.default_port {
        if port == 0 {
            return Err(AppError::Message(
                "默认 SSH 端口必须在 1 到 65535 之间".into(),
            ));
        }
    }
    if let Some(jump_id) = input
        .jump_server_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        db.get_server(jump_id)?;
    }
    if let Some(profile_id) = input
        .auth_profile_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        db.get_auth_profile(profile_id)?;
    }
    Ok(())
}

fn auth_name(method: &crate::models::AuthMethod) -> &'static str {
    match method {
        crate::models::AuthMethod::Password => "password",
        crate::models::AuthMethod::Key => "key",
        crate::models::AuthMethod::Agent => "agent",
    }
}

fn json_string<T: serde::Serialize>(value: &T) -> AppResult<String> {
    serde_json::to_string(value).map_err(|error| AppError::Message(error.to_string()))
}

fn runbook_trigger(trigger: RunbookTrigger) -> &'static str {
    match trigger {
        RunbookTrigger::Manual => "manual",
        RunbookTrigger::Scheduled => "scheduled",
        RunbookTrigger::Retry => "retry",
    }
}

fn runbook_execution_status(status: RunbookExecutionStatus) -> &'static str {
    match status {
        RunbookExecutionStatus::Queued => "queued",
        RunbookExecutionStatus::Running => "running",
        RunbookExecutionStatus::Succeeded => "succeeded",
        RunbookExecutionStatus::Failed => "failed",
        RunbookExecutionStatus::Partial => "partial",
        RunbookExecutionStatus::Cancelled => "cancelled",
        RunbookExecutionStatus::TimedOut => "timed_out",
        RunbookExecutionStatus::Interrupted => "interrupted",
    }
}

fn parse_enum<T: serde::de::DeserializeOwned>(value: &str) -> rusqlite::Result<T> {
    serde_json::from_str(&format!("\"{value}\"")).map_err(|_| rusqlite::Error::InvalidQuery)
}

fn row_to_runbook_schedule(row: &Row<'_>) -> rusqlite::Result<RunbookSchedule> {
    let server_ids: String = row.get(3)?;
    Ok(RunbookSchedule {
        id: row.get(0)?,
        runbook_id: row.get(1)?,
        runbook_name: row.get(2)?,
        server_ids: serde_json::from_str(&server_ids).map_err(|_| rusqlite::Error::InvalidQuery)?,
        interval_minutes: row.get(4)?,
        enabled: row.get(5)?,
        concurrency: row.get(6)?,
        canary_count: row.get(7)?,
        timeout_seconds: row.get(8)?,
        next_run_at: row.get(9)?,
        last_run_at: row.get(10)?,
        last_execution_id: row.get(11)?,
        last_error: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

fn row_to_runbook_execution(row: &Row<'_>) -> rusqlite::Result<RunbookExecution> {
    let target_ids: String = row.get(7)?;
    let result: Option<String> = row.get(17)?;
    Ok(RunbookExecution {
        id: row.get(0)?,
        runbook_id: row.get(1)?,
        runbook_name: row.get(2)?,
        schedule_id: row.get(3)?,
        retry_of: row.get(4)?,
        trigger: parse_enum(&row.get::<_, String>(5)?)?,
        status: parse_enum(&row.get::<_, String>(6)?)?,
        target_ids: serde_json::from_str(&target_ids).map_err(|_| rusqlite::Error::InvalidQuery)?,
        concurrency: row.get(8)?,
        canary_count: row.get(9)?,
        timeout_seconds: row.get(10)?,
        success_count: row.get(11)?,
        failure_count: row.get(12)?,
        skipped_count: row.get(13)?,
        created_at: row.get(14)?,
        started_at: row.get(15)?,
        finished_at: row.get(16)?,
        result: result
            .map(|value| serde_json::from_str(&value))
            .transpose()
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        error: row.get(18)?,
    })
}

fn row_to_data_protection_policy(row: &Row<'_>) -> rusqlite::Result<DataProtectionPolicy> {
    let paths: String = row.get(6)?;
    Ok(DataProtectionPolicy {
        id: row.get(0)?,
        server_id: row.get(1)?,
        name: row.get(2)?,
        repository_kind: parse_enum(&row.get::<_, String>(3)?)?,
        repository: row.get(4)?,
        source_kind: parse_enum(&row.get::<_, String>(5)?)?,
        paths: serde_json::from_str(&paths).map_err(|_| rusqlite::Error::InvalidQuery)?,
        volume_name: row.get(7)?,
        database_container: row.get(8)?,
        database_name: row.get(9)?,
        database_user: row.get(10)?,
        keep_last: row.get(11)?,
        has_repository_password: row.get(12)?,
        has_s3_credentials: row.get(13)?,
        has_database_password: row.get(14)?,
        schedule_enabled: row.get(15)?,
        schedule_interval_minutes: row.get(16)?,
        next_run_at: row.get(17)?,
        last_run_at: row.get(18)?,
        last_status: row.get(19)?,
        last_error: row.get(20)?,
        created_at: row.get(21)?,
        updated_at: row.get(22)?,
    })
}

fn validate_backup_schedule_interval(enabled: bool, minutes: u32) -> AppResult<u32> {
    if !enabled {
        return Ok(minutes.clamp(60, 43_200));
    }
    if !(60..=43_200).contains(&minutes) {
        return Err(AppError::Message(
            "定时备份间隔必须在 1 小时（60 分钟）到 30 天之间".into(),
        ));
    }
    Ok(minutes)
}

fn notification_channel_kind(kind: NotificationChannelKind) -> &'static str {
    match kind {
        NotificationChannelKind::Telegram => "telegram",
        NotificationChannelKind::Feishu => "feishu",
        NotificationChannelKind::Dingtalk => "dingtalk",
        NotificationChannelKind::Wecom => "wecom",
        NotificationChannelKind::Ntfy => "ntfy",
        NotificationChannelKind::Bark => "bark",
        NotificationChannelKind::Webhook => "webhook",
    }
}

fn notification_event_kind(kind: NotificationEventKind) -> &'static str {
    match kind {
        NotificationEventKind::HostAlert => "host_alert",
        NotificationEventKind::ServiceAlert => "service_alert",
        NotificationEventKind::CertificateExpiry => "certificate_expiry",
        NotificationEventKind::BackupFailure => "backup_failure",
        NotificationEventKind::RunbookFailure => "runbook_failure",
        NotificationEventKind::ServerExpiry => "server_expiry",
    }
}

fn notification_severity(severity: NotificationSeverity) -> &'static str {
    match severity {
        NotificationSeverity::Info => "info",
        NotificationSeverity::Warning => "warning",
        NotificationSeverity::Critical => "critical",
    }
}

fn row_to_notification_channel(row: &Row<'_>) -> rusqlite::Result<NotificationChannel> {
    let raw_events: String = row.get(5)?;
    Ok(NotificationChannel {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: parse_enum(&row.get::<_, String>(2)?)?,
        enabled: row.get(3)?,
        target: row.get(4)?,
        events: serde_json::from_str(&raw_events).unwrap_or_default(),
        min_severity: parse_enum(&row.get::<_, String>(6)?)?,
        quiet_start: row.get(7)?,
        quiet_end: row.get(8)?,
        has_secret: row.get(9)?,
        has_signing_secret: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn row_to_notification_delivery(row: &Row<'_>) -> rusqlite::Result<NotificationDelivery> {
    Ok(NotificationDelivery {
        id: row.get(0)?,
        channel_id: row.get(1)?,
        channel_name: row.get(2)?,
        event_kind: parse_enum(&row.get::<_, String>(3)?)?,
        severity: parse_enum(&row.get::<_, String>(4)?)?,
        title: row.get(5)?,
        message: row.get(6)?,
        success: row.get(7)?,
        error: row.get(8)?,
        attempts: row.get(9)?,
        created_at: row.get(10)?,
    })
}

fn parse_expiry_date(raw: &str) -> Option<chrono::NaiveDate> {
    let trimmed = raw.trim();
    if trimmed.len() >= 10 {
        if let Ok(date) = chrono::NaiveDate::parse_from_str(&trimmed[..10], "%Y-%m-%d") {
            return Some(date);
        }
    }
    chrono::DateTime::parse_from_rfc3339(trimmed)
        .ok()
        .map(|dt| dt.date_naive())
}

fn row_to_data_protection_run(row: &Row<'_>) -> rusqlite::Result<DataProtectionRun> {
    Ok(DataProtectionRun {
        id: row.get(0)?,
        policy_id: row.get(1)?,
        action: parse_enum(&row.get::<_, String>(2)?)?,
        status: parse_enum(&row.get::<_, String>(3)?)?,
        snapshot_id: row.get(4)?,
        target: row.get(5)?,
        output: row.get(6)?,
        error: row.get(7)?,
        started_at: row.get(8)?,
        finished_at: row.get(9)?,
        duration_ms: row.get::<_, i64>(10)?.max(0) as u64,
    })
}

fn row_to_cloud_account(row: &Row<'_>) -> rusqlite::Result<CloudAccount> {
    Ok(CloudAccount {
        id: row.get(0)?,
        name: row.get(1)?,
        provider: parse_enum(&row.get::<_, String>(2)?)?,
        has_token: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        last_synced_at: row.get(6)?,
    })
}

fn row_to_auth_profile(row: &Row<'_>) -> rusqlite::Result<AuthProfile> {
    Ok(AuthProfile {
        id: row.get(0)?,
        name: row.get(1)?,
        auth_method: parse_enum(&row.get::<_, String>(2)?)?,
        private_key_path: row.get(3)?,
        has_password: row.get(4)?,
        has_key_passphrase: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn row_to_server_group(row: &Row<'_>) -> rusqlite::Result<ServerGroup> {
    Ok(ServerGroup {
        id: row.get(0)?,
        name: row.get(1)?,
        jump_server_id: row.get(2)?,
        auth_profile_id: row.get(3)?,
        default_port: row.get(4)?,
        default_username: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn row_to_server(row: &Row<'_>) -> rusqlite::Result<Server> {
    let method: String = row.get(5)?;
    let tags: String = row.get(8)?;
    Ok(Server {
        id: row.get(0)?,
        name: row.get(1)?,
        host: row.get(2)?,
        port: row.get(3)?,
        username: row.get(4)?,
        auth_method: serde_json::from_str(&format!("\"{method}\""))
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        private_key_path: row.get(6)?,
        group_name: row.get(7)?,
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        provider: row.get(9)?,
        region: row.get(10)?,
        notes: row.get(11)?,
        cost: row.get(12)?,
        currency: row.get(13)?,
        expires_at: row.get(14)?,
        jump_server_id: row.get(15)?,
        auth_profile_id: row.get(16)?,
        cloud_account_id: row.get(17)?,
        cloud_resource_id: row.get(18)?,
        inherit_port: row.get(19)?,
        inherit_username: row.get(20)?,
        inherit_jump: row.get(21)?,
        inherit_auth: row.get(22)?,
        created_at: row.get(23)?,
        updated_at: row.get(24)?,
        has_password: row.get(25)?,
        has_key_passphrase: row.get(26)?,
        jump_chain: row
            .get::<_, Option<String>>(27)?
            .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
            .unwrap_or_default(),
    })
}

fn row_to_runbook(row: &Row<'_>) -> rusqlite::Result<Runbook> {
    let steps: String = row.get(3)?;
    Ok(Runbook {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        steps: serde_json::from_str(&steps).map_err(|_| rusqlite::Error::InvalidQuery)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

fn row_to_port_forward(row: &Row<'_>) -> rusqlite::Result<PortForwardRule> {
    let kind: String = row.get(3)?;
    Ok(PortForwardRule {
        id: row.get(0)?,
        name: row.get(1)?,
        server_id: row.get(2)?,
        kind: serde_json::from_str(&format!("\"{kind}\""))
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        listen_address: row.get(4)?,
        listen_port: row.get(5)?,
        destination_host: row.get(6)?,
        destination_port: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn row_to_service_monitor(row: &Row<'_>) -> rusqlite::Result<ServiceMonitor> {
    let kind: String = row.get(2)?;
    Ok(ServiceMonitor {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: serde_json::from_str(&format!("\"{kind}\""))
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        target: row.get(3)?,
        port: row.get(4)?,
        server_id: row.get(5)?,
        interval_seconds: row.get(6)?,
        timeout_seconds: row.get(7)?,
        expected_status: row.get(8)?,
        keyword: row.get(9)?,
        failure_threshold: row.get(10)?,
        certificate_warning_days: row.get(11)?,
        enabled: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

fn row_to_service_sample(row: &Row<'_>) -> rusqlite::Result<ServiceSample> {
    Ok(ServiceSample {
        id: row.get(0)?,
        monitor_id: row.get(1)?,
        success: row.get(2)?,
        latency_ms: row.get(3)?,
        status_code: row.get(4)?,
        detail: row.get(5)?,
        certificate_expires_at: row.get(6)?,
        certificate_days_remaining: row.get(7)?,
        checked_at: row.get(8)?,
    })
}

fn row_to_service_event(row: &Row<'_>) -> rusqlite::Result<ServiceEvent> {
    let kind: String = row.get(3)?;
    let status: String = row.get(4)?;
    Ok(ServiceEvent {
        id: row.get(0)?,
        monitor_id: row.get(1)?,
        monitor_name: row.get(2)?,
        kind: serde_json::from_str(&format!("\"{kind}\""))
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        status: serde_json::from_str(&format!("\"{status}\""))
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        message: row.get(5)?,
        created_at: row.get(6)?,
    })
}

fn row_to_alert_event(row: &Row<'_>) -> rusqlite::Result<AlertEvent> {
    let kind: String = row.get(3)?;
    let status: String = row.get(4)?;
    Ok(AlertEvent {
        id: row.get(0)?,
        server_id: row.get(1)?,
        server_name: row.get(2)?,
        kind: serde_json::from_str(&format!("\"{kind}\""))
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        status: serde_json::from_str(&format!("\"{status}\""))
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        message: row.get(5)?,
        value: row.get(6)?,
        threshold: row.get(7)?,
        created_at: row.get(8)?,
    })
}

fn row_to_log(row: &Row<'_>) -> rusqlite::Result<OperationLog> {
    Ok(OperationLog {
        id: row.get(0)?,
        server_id: row.get(1)?,
        server_name: row.get(2)?,
        action: row.get(3)?,
        success: row.get(4)?,
        summary: row.get(5)?,
        created_at: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        AuthMethod, RunbookStepInput, ServiceMonitorInput, ServiceMonitorKind, ServiceProbe,
    };

    fn sample_server(name: &str, host: &str) -> ServerInput {
        ServerInput {
            name: name.into(),
            host: host.into(),
            port: 22,
            username: "root".into(),
            auth_method: AuthMethod::Agent,
            private_key_path: None,
            password: None,
            key_passphrase: None,
            group_name: None,
            tags: vec![],
            provider: None,
            region: None,
            notes: None,
            cost: None,
            currency: None,
            expires_at: None,
            jump_server_id: None,
            jump_chain: vec![],
            auth_profile_id: None,
            cloud_account_id: None,
            cloud_resource_id: None,
            inherit_port: false,
            inherit_username: false,
            inherit_jump: false,
            inherit_auth: false,
        }
    }

    #[test]
    fn schema_version_and_group_jump_guard() {
        let db = Database::in_memory().expect("in-memory db");
        let version: i64 = db
            .connection()
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);

        let jump = db
            .create_server(&sample_server("bastion", "10.0.0.1"))
            .expect("create bastion");
        let group = db
            .create_server_group(&ServerGroupInput {
                name: "prod".into(),
                jump_server_id: Some(jump.id.clone()),
                auth_profile_id: None,
                default_port: Some(2222),
                default_username: Some("ops".into()),
            })
            .expect("create group");

        let err = db.ensure_server_deletable(&jump.id).unwrap_err();
        assert!(err.to_string().contains("默认跳板机"));

        // Renaming a group cascades to servers belonging to it.
        let mut app_input = sample_server("app-1", "10.0.0.2");
        app_input.group_name = Some("prod".into());
        let app = db.create_server(&app_input).unwrap();

        db.update_server_group(
            &group.id,
            &ServerGroupInput {
                name: "production".into(),
                jump_server_id: None,
                auth_profile_id: None,
                default_port: Some(2222),
                default_username: Some("ops".into()),
            },
        )
        .unwrap();

        let reloaded = db.get_server(&app.id).unwrap();
        assert_eq!(reloaded.group_name.as_deref(), Some("production"));
        // After detaching bastion from group, it can be deleted.
        db.ensure_server_deletable(&jump.id).unwrap();
    }

    #[test]
    fn delete_server_cleans_runbook_schedules() {
        let db = Database::in_memory().unwrap();
        let s1 = db
            .create_server(&sample_server("node-1", "10.0.0.1"))
            .unwrap();
        let s2 = db
            .create_server(&sample_server("node-2", "10.0.0.2"))
            .unwrap();
        let runbook = db
            .create_runbook(&RunbookInput {
                name: "check".into(),
                description: None,
                steps: vec![RunbookStepInput {
                    name: "uname".into(),
                    command: "uname -a".into(),
                }],
            })
            .unwrap();
        let schedule = db
            .create_runbook_schedule(&RunbookScheduleInput {
                runbook_id: runbook.id.clone(),
                server_ids: vec![s1.id.clone(), s2.id.clone()],
                interval_minutes: 60,
                enabled: true,
                concurrency: 2,
                canary_count: 0,
                timeout_seconds: 60,
            })
            .unwrap();

        db.delete_server(&s1.id).unwrap();
        let schedules = db.list_runbook_schedules().unwrap();
        let updated = schedules.iter().find(|item| item.id == schedule.id).unwrap();
        assert_eq!(updated.server_ids, vec![s2.id.clone()]);
        assert!(updated.enabled);

        db.delete_server(&s2.id).unwrap();
        let schedules = db.list_runbook_schedules().unwrap();
        let updated = schedules.iter().find(|item| item.id == schedule.id).unwrap();
        assert!(updated.server_ids.is_empty());
        assert!(!updated.enabled);
    }

    #[test]
    fn service_monitor_single_query_uptime() {
        let db = Database::in_memory().unwrap();
        let monitor_view = db
            .create_service_monitor(&ServiceMonitorInput {
                name: "api".into(),
                kind: ServiceMonitorKind::Http,
                target: "https://example.com".into(),
                port: None,
                server_id: None,
                interval_seconds: 60,
                timeout_seconds: 10,
                expected_status: 200,
                keyword: None,
                failure_threshold: 2,
                certificate_warning_days: 14,
                enabled: true,
            })
            .unwrap();
        let monitor_id = monitor_view.monitor.id.clone();
        db.record_service_probe(
            &monitor_view.monitor,
            &ServiceProbe {
                success: true,
                latency_ms: 42,
                status_code: Some(200),
                detail: "ok".into(),
                certificate_expires_at: None,
                certificate_days_remaining: None,
            },
        )
        .unwrap();
        db.record_service_probe(
            &monitor_view.monitor,
            &ServiceProbe {
                success: false,
                latency_ms: 100,
                status_code: Some(500),
                detail: "err".into(),
                certificate_expires_at: None,
                certificate_days_remaining: None,
            },
        )
        .unwrap();
        let view = db.service_monitor_view(&monitor_id).unwrap();
        assert_eq!(view.uptime_24h, Some(50.0));
        assert_eq!(view.uptime_7d, Some(50.0));
        assert_eq!(view.uptime_30d, Some(50.0));
    }

    #[test]
    fn notification_channels_deliveries_and_action_center() {
        let db = Database::in_memory().unwrap();
        let mut s = sample_server("expiring-vps", "10.0.0.9");
        s.expires_at = Some((Utc::now() + Duration::days(3)).format("%Y-%m-%d").to_string());
        let server = db.create_server(&s).unwrap();

        let channel = db
            .create_notification_channel(
                "ch-1",
                &NotificationChannelInput {
                    name: "Ops Telegram".into(),
                    kind: NotificationChannelKind::Telegram,
                    enabled: true,
                    target: "123456".into(),
                    secret: Some("bot-token".into()),
                    signing_secret: None,
                    clear_signing_secret: false,
                    events: vec![NotificationEventKind::HostAlert, NotificationEventKind::BackupFailure],
                    min_severity: NotificationSeverity::Warning,
                    quiet_start: Some("23:00".into()),
                    quiet_end: Some("08:00".into()),
                },
            )
            .unwrap();
        assert!(channel.has_secret);
        assert_eq!(db.list_notification_channels().unwrap().len(), 1);

        db.record_notification_delivery(
            &channel.id,
            &channel.name,
            NotificationEventKind::HostAlert,
            NotificationSeverity::Critical,
            "CPU",
            "high",
            true,
            None,
            1,
        )
        .unwrap();
        assert_eq!(db.list_notification_deliveries(10).unwrap().len(), 1);

        let policy = db
            .create_data_protection_policy(
                "pol-1",
                &server.id,
                &DataProtectionInput {
                    name: "Daily DB".into(),
                    repository_kind: ResticRepositoryKind::Local,
                    repository: "/srv/restic".into(),
                    repository_password: Some("strong-secret".into()),
                    s3_access_key_id: None,
                    s3_secret_access_key: None,
                    source_kind: ProtectionSourceKind::Directory,
                    paths: vec!["/etc".into()],
                    volume_name: None,
                    database_container: None,
                    database_name: None,
                    database_user: None,
                    database_password: None,
                    keep_last: 7,
                    schedule_enabled: true,
                    schedule_interval_minutes: 360,
                },
            )
            .unwrap();
        assert!(policy.schedule_enabled);
        assert!(policy.next_run_at.is_some());

        let run = db
            .start_data_protection_run(&policy.id, DataProtectionAction::Backup, None)
            .unwrap();
        db.finish_data_protection_run(
            &run.id,
            DataProtectionRunStatus::Failed,
            None,
            "",
            Some("connection refused"),
            100,
        )
        .unwrap();

        let summary = db.action_center_summary().unwrap();
        assert_eq!(summary.expiring_servers.len(), 1);
        assert_eq!(summary.failed_backups.len(), 1);
        assert_eq!(summary.enabled_notification_channels, 1);
        assert_eq!(summary.scheduled_backup_policies, 1);

        let bench = crate::benchmark::parse_benchmark_output(
            &server.id,
            &server.name,
            Some(40),
            1200,
            "__CPU_MODEL__=TestCPU\n__CPU_CORES__=2\n__CPU_MS__=200\n__DISK_MS__=200\n",
        );
        let mut bench = bench;
        bench.id = "b-1".into();
        bench.created_at = Utc::now().to_rfc3339();
        db.save_server_benchmark(&bench).unwrap();
        assert_eq!(db.list_server_benchmarks(Some(&server.id)).unwrap().len(), 1);
        assert_eq!(db.list_server_benchmarks(None).unwrap().len(), 1);
    }
}
