use std::sync::Mutex;

use chrono::Utc;
use rusqlite::{params, Connection, Row};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::{
    OperationLog, PortForwardInput, PortForwardRule, Server, ServerInput, Snippet, SnippetInput,
};

pub struct Database {
    connection: Mutex<Connection>,
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
        Ok(Self {
            connection: Mutex::new(connection),
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
                    expires_at, created_at, updated_at, has_password, has_key_passphrase
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
                        expires_at, created_at, updated_at, has_password, has_key_passphrase
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
        connection.execute(
            "INSERT INTO servers (
                id, name, host, port, username, auth_method, private_key_path,
                group_name, tags, provider, region, notes, cost, currency, expires_at,
                created_at, updated_at, has_password, has_key_passphrase
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
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
        connection.execute(
            "UPDATE servers SET name = ?2, host = ?3, port = ?4, username = ?5,
                auth_method = ?6, private_key_path = ?7, group_name = ?8, tags = ?9,
                provider = ?10, region = ?11, notes = ?12, cost = ?13, currency = ?14,
                expires_at = ?15, updated_at = ?16, has_password = ?17, has_key_passphrase = ?18
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
                Utc::now().to_rfc3339(),
                has_password,
                has_key_passphrase,
            ],
        )?;
        drop(connection);
        self.get_server(id)
    }

    pub fn delete_server(&self, id: &str) -> AppResult<()> {
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
}

fn validate_port_forward(input: &PortForwardInput) -> AppResult<()> {
    if input.name.trim().is_empty() {
        return Err(AppError::Message("规则名称不能为空".into()));
    }
    if input.kind == crate::models::PortForwardKind::Local
        && input.destination_host.trim().is_empty()
    {
        return Err(AppError::Message("本地转发的目标地址不能为空".into()));
    }
    if input.listen_port == 0
        || (input.kind == crate::models::PortForwardKind::Local && input.destination_port == 0)
    {
        return Err(AppError::Message(
            "监听端口必须在 1 到 65535 之间，本地转发还需要有效的目标端口".into(),
        ));
    }
    input
        .listen_address
        .parse::<std::net::IpAddr>()
        .map_err(|_| AppError::Message("监听地址必须是有效的 IP 地址，例如 127.0.0.1".into()))?;
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
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
        has_password: row.get(17)?,
        has_key_passphrase: row.get(18)?,
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
