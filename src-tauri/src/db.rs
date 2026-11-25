use std::sync::{Arc, Mutex};

use chrono::{Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::{
    AlertEvent, AlertKind, AlertStatus, MetricSample, MonitoringSettings, OperationLog,
    PortForwardInput, PortForwardRule, Runbook, RunbookInput, RunbookStep, Server, ServerInput,
    ServerMetrics, ServiceEvent, ServiceEventKind, ServiceMonitor, ServiceMonitorInput,
    ServiceMonitorKind, ServiceMonitorView, ServiceProbe, ServiceSample, Snippet, SnippetInput,
    WorkspaceBackup,
};

#[derive(Clone)]
pub struct Database {
    connection: Arc<Mutex<Connection>>,
}

impl Database {
    pub fn new(path: &std::path::Path) -> AppResult<Self> {
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
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
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                has_password INTEGER NOT NULL DEFAULT 0,
                has_key_passphrase INTEGER NOT NULL DEFAULT 0
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
             CREATE TABLE IF NOT EXISTS app_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
             );",
        )?;
        let has_forward_kind = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('port_forwards') WHERE name = 'kind')",
            [],
            |row| row.get::<_, bool>(0),
        )?;
        if !has_forward_kind {
            connection.execute(
                "ALTER TABLE port_forwards ADD COLUMN kind TEXT NOT NULL DEFAULT 'local'",
                [],
            )?;
        }
        let has_jump_server = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('servers') WHERE name = 'jump_server_id')",
            [],
            |row| row.get::<_, bool>(0),
        )?;
        if !has_jump_server {
            connection.execute("ALTER TABLE servers ADD COLUMN jump_server_id TEXT", [])?;
        }
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
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
                    expires_at, jump_server_id, created_at, updated_at, has_password, has_key_passphrase
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
                        expires_at, jump_server_id, created_at, updated_at, has_password, has_key_passphrase
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
        validate_jump_server(&connection, &id, input.jump_server_id.as_deref())?;
        connection.execute(
            "INSERT INTO servers (
                id, name, host, port, username, auth_method, private_key_path,
                group_name, tags, provider, region, notes, cost, currency, expires_at,
                jump_server_id, created_at, updated_at, has_password, has_key_passphrase
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
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
                now,
                now,
                input.password.as_deref().is_some_and(|value| !value.is_empty()),
                input.key_passphrase.as_deref().is_some_and(|value| !value.is_empty()),
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
        validate_jump_server(&connection, id, input.jump_server_id.as_deref())?;
        connection.execute(
            "UPDATE servers SET name = ?2, host = ?3, port = ?4, username = ?5,
                auth_method = ?6, private_key_path = ?7, group_name = ?8, tags = ?9,
                provider = ?10, region = ?11, notes = ?12, cost = ?13, currency = ?14,
                expires_at = ?15, jump_server_id = ?16, updated_at = ?17,
                has_password = ?18, has_key_passphrase = ?19
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
                Utc::now().to_rfc3339(),
                has_password,
                has_key_passphrase,
            ],
        )?;
        drop(connection);
        self.get_server(id)
    }

    pub fn ensure_server_deletable(&self, id: &str) -> AppResult<()> {
        let connection = self.connection()?;
        let dependents = connection.query_row(
            "SELECT COUNT(*) FROM servers WHERE jump_server_id = ?1",
            [id],
            |row| row.get::<_, i64>(0),
        )?;
        if dependents > 0 {
            return Err(AppError::Message(format!(
                "这台 VPS 正被 {dependents} 台服务器用作跳板机，请先修改它们的连接路由"
            )));
        }
        Ok(())
    }

    pub fn delete_server(&self, id: &str) -> AppResult<()> {
        self.ensure_server_deletable(id)?;
        self.connection()?
            .execute("DELETE FROM servers WHERE id = ?1", [id])?;
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
    Ok(ServiceMonitorView {
        monitor: monitor.clone(),
        latest,
        active_events,
        consecutive_failures,
        uptime_24h: service_uptime(connection, &monitor.id, Duration::hours(24))?,
        uptime_7d: service_uptime(connection, &monitor.id, Duration::days(7))?,
        uptime_30d: service_uptime(connection, &monitor.id, Duration::days(30))?,
    })
}

fn service_uptime(
    connection: &Connection,
    monitor_id: &str,
    duration: Duration,
) -> rusqlite::Result<Option<f64>> {
    let cutoff = (Utc::now() - duration).to_rfc3339();
    let (total, successful): (i64, Option<i64>) = connection.query_row(
        "SELECT COUNT(*), SUM(success) FROM service_samples WHERE monitor_id = ?1 AND checked_at >= ?2",
        params![monitor_id, cutoff],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok((total > 0).then(|| successful.unwrap_or_default() as f64 / total as f64 * 100.0))
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

fn validate_jump_server(
    connection: &Connection,
    server_id: &str,
    jump_server_id: Option<&str>,
) -> AppResult<()> {
    let Some(jump_server_id) = clean(jump_server_id) else {
        return Ok(());
    };
    if jump_server_id == server_id {
        return Err(AppError::Message("服务器不能使用自身作为跳板机".into()));
    }
    let dependents = connection.query_row(
        "SELECT COUNT(*) FROM servers WHERE jump_server_id = ?1",
        [server_id],
        |row| row.get::<_, i64>(0),
    )?;
    if dependents > 0 {
        return Err(AppError::Message(
            "这台 VPS 正作为其他服务器的跳板机，不能再配置上级跳板".into(),
        ));
    }
    let nested = connection
        .query_row(
            "SELECT jump_server_id FROM servers WHERE id = ?1",
            [&jump_server_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => AppError::Message("所选跳板机不存在".into()),
            other => other.into(),
        })?;
    if nested.is_some() {
        return Err(AppError::Message(
            "当前支持一级跳板，请选择直接联网的 VPS 作为跳板机".into(),
        ));
    }
    Ok(())
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

fn auth_name(method: &crate::models::AuthMethod) -> &'static str {
    match method {
        crate::models::AuthMethod::Password => "password",
        crate::models::AuthMethod::Key => "key",
        crate::models::AuthMethod::Agent => "agent",
    }
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
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
        has_password: row.get(18)?,
        has_key_passphrase: row.get(19)?,
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
