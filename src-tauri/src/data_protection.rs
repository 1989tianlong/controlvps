use serde::Deserialize;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::db::Database;
use crate::error::{AppError, AppResult};
use crate::models::{
    DataProtectionAction, DataProtectionInput, DataProtectionOverview, DataProtectionPolicy,
    DataProtectionRestoreResult, DataProtectionRun, DataProtectionRunStatus, ProtectionSourceKind,
    ResticRepositoryKind, ResticSnapshot,
};
use crate::runbook;
use crate::ssh::{self, SshConnection};
use crate::utils::{self, shell_quote};

pub fn overview(
    db: &Database,
    server_id: &str,
    known_hosts: &Path,
    allow_unknown_host: bool,
) -> AppResult<DataProtectionOverview> {
    let connection = connect(db, server_id, known_hosts, allow_unknown_host)?;
    let version = ssh::execute(
        &connection.session,
        "if command -v restic >/dev/null 2>&1; then restic version; else printf '未安装 Restic'; fi",
    )?;
    Ok(DataProtectionOverview {
        restic_version: version.stdout.trim().to_string(),
        policies: db.list_data_protection_policies(server_id)?,
    })
}

pub fn create_policy(
    db: &Database,
    server_id: &str,
    input: &DataProtectionInput,
) -> AppResult<DataProtectionPolicy> {
    let id = Uuid::new_v4().to_string();
    save_secrets(&id, input)?;
    match db.create_data_protection_policy(&id, server_id, input) {
        Ok(policy) => Ok(policy),
        Err(error) => {
            delete_policy_secrets(&id);
            Err(error)
        }
    }
}

pub fn update_policy(
    db: &Database,
    id: &str,
    input: &DataProtectionInput,
) -> AppResult<DataProtectionPolicy> {
    db.get_data_protection_policy(id)?;
    save_secrets(id, input)?;
    if input.repository_kind == ResticRepositoryKind::Local {
        delete_secret(id, "s3-access-key")?;
        delete_secret(id, "s3-secret-key")?;
    }
    db.update_data_protection_policy(id, input)
}

pub fn delete_policy(db: &Database, id: &str) -> AppResult<()> {
    db.get_data_protection_policy(id)?;
    db.delete_data_protection_policy(id)?;
    delete_policy_secrets(id);
    Ok(())
}

pub fn delete_policy_secrets_by_id(id: &str) {
    delete_policy_secrets(id);
}

pub fn initialize(
    db: &Database,
    policy_id: &str,
    known_hosts: &Path,
    allow_unknown_host: bool,
) -> AppResult<DataProtectionRun> {
    execute_record(
        db,
        policy_id,
        DataProtectionAction::Initialize,
        None,
        known_hosts,
        allow_unknown_host,
        |connection, policy| {
            let context = ResticContext::load(policy)?;
            let output = context.run(&connection.session, "init", false)?;
            Ok((None, display_output(&output)))
        },
    )
}

pub fn check(
    db: &Database,
    policy_id: &str,
    known_hosts: &Path,
    allow_unknown_host: bool,
) -> AppResult<DataProtectionRun> {
    execute_record(
        db,
        policy_id,
        DataProtectionAction::Check,
        None,
        known_hosts,
        allow_unknown_host,
        |connection, policy| {
            let context = ResticContext::load(policy)?;
            let output = context.run(&connection.session, "check", false)?;
            Ok((None, display_output(&output)))
        },
    )
}

pub fn backup(
    db: &Database,
    policy_id: &str,
    known_hosts: &Path,
    allow_unknown_host: bool,
) -> AppResult<DataProtectionRun> {
    execute_record(
        db,
        policy_id,
        DataProtectionAction::Backup,
        None,
        known_hosts,
        allow_unknown_host,
        |connection, policy| {
            let context = ResticContext::load(policy)?;
            let (snapshot_id, mut output) = create_snapshot(connection, policy, &context)?;
            let retention = format!(
                "forget --keep-last {} --prune --tag {}",
                policy.keep_last,
                shell_quote(&format!("policy:{}", policy.id))
            );
            match context.run(&connection.session, &retention, false) {
                Ok(forget) => {
                    output.push_str(&sanitize(&display_output(&forget), &context.secrets));
                }
                Err(error) => {
                    output.push_str(&format!(
                        "\n警告：快照已创建，但旧快照清理失败：{}",
                        sanitize(&error.to_string(), &context.secrets)
                    ));
                }
            }
            Ok((Some(snapshot_id), sanitize(&output, &context.secrets)))
        },
    )
}

pub fn snapshots(
    db: &Database,
    policy_id: &str,
    known_hosts: &Path,
    allow_unknown_host: bool,
) -> AppResult<Vec<ResticSnapshot>> {
    let policy = db.get_data_protection_policy(policy_id)?;
    let connection = connect(db, &policy.server_id, known_hosts, allow_unknown_host)?;
    let context = ResticContext::load(&policy)?;
    list_snapshots(&connection, &policy, &context)
}

pub fn restore(
    db: &Database,
    policy_id: &str,
    snapshot_id: &str,
    target: Option<&str>,
    known_hosts: &Path,
    allow_unknown_host: bool,
) -> AppResult<DataProtectionRestoreResult> {
    if snapshot_id.len() < 8
        || snapshot_id.len() > 64
        || !snapshot_id
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(AppError::Message("Restic 快照 ID 无效".into()));
    }
    let target = match target.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) if value.starts_with('/') && value != "/" && !value.contains('\n') => {
            value.to_string()
        }
        Some(_) => {
            return Err(AppError::Message(
                "恢复目标必须是新的绝对路径，不能是根目录".into(),
            ));
        }
        None => format!("/tmp/controlvps-restore-{}", Uuid::new_v4()),
    };
    let policy = db.get_data_protection_policy(policy_id)?;
    let run =
        db.start_data_protection_run(policy_id, DataProtectionAction::Restore, Some(&target))?;
    let started = Instant::now();
    let secrets = ResticContext::load(&policy)
        .map(|ctx| ctx.secrets)
        .unwrap_or_default();
    let result = (|| -> AppResult<String> {
        let connection = connect(db, &policy.server_id, known_hosts, allow_unknown_host)?;
        let context = ResticContext::load(&policy)?;
        let snapshots = list_snapshots(&connection, &policy, &context)?;
        if !snapshots.iter().any(|snapshot| snapshot.id == snapshot_id) {
            return Err(AppError::Message(
                "快照不存在或不属于当前策略，已停止恢复".into(),
            ));
        }
        let prepare = format!(
            "if [ -e {target} ] || [ -L {target} ]; then printf '恢复目标已存在，拒绝覆盖\\n' >&2; exit 9; fi; mkdir -p -m 700 -- {target}",
            target = shell_quote(&target)
        );
        run_checked(&connection, &prepare, &context.secrets)?;
        let output = context.run(
            &connection.session,
            &format!(
                "restore {} --target {}",
                shell_quote(snapshot_id),
                shell_quote(&target)
            ),
            true,
        )?;
        Ok(sanitize(&display_output(&output), &context.secrets))
    })();
    match result {
        Ok(output) => {
            db.finish_data_protection_run(
                &run.id,
                DataProtectionRunStatus::Succeeded,
                Some(snapshot_id),
                &output,
                None,
                started.elapsed().as_millis() as u64,
            )?;
            Ok(DataProtectionRestoreResult {
                snapshot_id: snapshot_id.to_string(),
                target,
                output,
            })
        }
        Err(error) => {
            let message = sanitize(&error.to_string(), &secrets);
            let _ = db.finish_data_protection_run(
                &run.id,
                DataProtectionRunStatus::Failed,
                Some(snapshot_id),
                "",
                Some(&message),
                started.elapsed().as_millis() as u64,
            );
            Err(AppError::Message(message))
        }
    }
}

fn execute_record(
    db: &Database,
    policy_id: &str,
    action: DataProtectionAction,
    target: Option<&str>,
    known_hosts: &Path,
    allow_unknown_host: bool,
    operation: impl FnOnce(&SshConnection, &DataProtectionPolicy) -> AppResult<(Option<String>, String)>,
) -> AppResult<DataProtectionRun> {
    let policy = db.get_data_protection_policy(policy_id)?;
    let run = db.start_data_protection_run(policy_id, action, target)?;
    let started = Instant::now();
    let result = connect(db, &policy.server_id, known_hosts, allow_unknown_host)
        .and_then(|connection| operation(&connection, &policy));
    match result {
        Ok((snapshot_id, output)) => {
            db.finish_data_protection_run(
                &run.id,
                DataProtectionRunStatus::Succeeded,
                snapshot_id.as_deref(),
                &output,
                None,
                started.elapsed().as_millis() as u64,
            )?;
        }
        Err(error) => {
            let message = error.to_string();
            db.finish_data_protection_run(
                &run.id,
                DataProtectionRunStatus::Failed,
                None,
                "",
                Some(&message),
                started.elapsed().as_millis() as u64,
            )?;
            return Err(AppError::Message(message));
        }
    }
    db.get_data_protection_run(&run.id)
}

fn create_snapshot(
    connection: &SshConnection,
    policy: &DataProtectionPolicy,
    context: &ResticContext,
) -> AppResult<(String, String)> {
    let tags = format!(
        "--tag controlvps --tag {} --tag {}",
        shell_quote(&format!("policy:{}", policy.id)),
        shell_quote(&format!("source:{}", source_name(policy.source_kind)))
    );
    let output = match policy.source_kind {
        ProtectionSourceKind::Directory => {
            let paths = policy
                .paths
                .iter()
                .map(|path| shell_quote(path))
                .collect::<Vec<_>>()
                .join(" ");
            context.run(
                &connection.session,
                &format!("backup {tags} -- {paths}"),
                true,
            )?
        }
        ProtectionSourceKind::DockerVolume => {
            let volume = policy.volume_name.as_deref().unwrap_or_default();
            let inspection = run_checked(
                connection,
                &format!(
                    "docker volume inspect --format '{{{{.Mountpoint}}}}' {}",
                    shell_quote(volume)
                ),
                &[],
            )?;
            let mountpoint = inspection.stdout.trim();
            if !mountpoint.starts_with('/') || mountpoint.contains('\n') {
                return Err(AppError::Message("Docker 卷挂载路径无效".into()));
            }
            context.run(
                &connection.session,
                &format!("backup {tags} -- {}", shell_quote(mountpoint)),
                true,
            )?
        }
        ProtectionSourceKind::Postgresql | ProtectionSourceKind::Mysql => {
            database_snapshot(connection, policy, context, &tags)?
        }
    };
    let snapshot_id = parse_snapshot_id(&output.stdout).ok_or_else(|| {
        AppError::Message("Restic 未返回快照 ID，请检查输出并核对仓库状态".into())
    })?;
    Ok((snapshot_id, sanitize(&display_output(&output), &context.secrets)))
}

fn database_snapshot(
    connection: &SshConnection,
    policy: &DataProtectionPolicy,
    context: &ResticContext,
    tags: &str,
) -> AppResult<ssh::ExecOutput> {
    let container = shell_quote(policy.database_container.as_deref().unwrap_or_default());
    let database = shell_quote(policy.database_name.as_deref().unwrap_or_default());
    let user = shell_quote(
        policy
            .database_user
            .as_deref()
            .unwrap_or(match policy.source_kind {
                ProtectionSourceKind::Mysql => "root",
                _ => "postgres",
            }),
    );
    let password = get_secret(&policy.id, "database-password")?;
    let mut secrets = context.secrets.clone();
    if let Some(pwd) = &password {
        secrets.push(pwd.clone());
    }
    let env_key = if policy.source_kind == ProtectionSourceKind::Mysql {
        "MYSQL_PWD"
    } else {
        "PGPASSWORD"
    };
    let mut script = String::new();
    script.push_str("set -eu\n");
    script.push_str(&context.export_lines);
    let env_file_arg = if let Some(pwd) = password.as_deref().filter(|v| !v.is_empty()) {
        script.push_str("umask 077\n");
        script.push_str("ENV_FILE=$(mktemp /tmp/controlvps-dbenv-XXXXXX)\n");
        script.push_str("STATUS_FILE=$(mktemp /tmp/controlvps-dbst-XXXXXX)\n");
        script.push_str("trap 'rm -f \"$ENV_FILE\" \"$STATUS_FILE\"' EXIT\n");
        script.push_str(&format!(
            "printf '%s\\n' {} > \"$ENV_FILE\"\n",
            shell_quote(&format!("{env_key}={pwd}"))
        ));
        " --env-file \"$ENV_FILE\" "
    } else {
        script.push_str("umask 077\n");
        script.push_str("STATUS_FILE=$(mktemp /tmp/controlvps-dbst-XXXXXX)\n");
        script.push_str("trap 'rm -f \"$STATUS_FILE\"' EXIT\n");
        " "
    };
    let dump_command = match policy.source_kind {
        ProtectionSourceKind::Postgresql => format!(
            "docker exec{env_file_arg}{container} pg_dump -U {user} -d {database} --clean --if-exists"
        ),
        ProtectionSourceKind::Mysql => format!(
            "docker exec{env_file_arg}{container} sh -c 'if command -v mysqldump >/dev/null 2>&1; then exec mysqldump \"$@\"; else exec mariadb-dump \"$@\"; fi' sh --single-transaction --quick -u {user} {database}"
        ),
        _ => unreachable!(),
    };
    // Stream dump directly into `restic backup --stdin` so unencrypted SQL never hits disk.
    script.push_str(&format!(
        "({dump_command}; printf '%s' $? > \"$STATUS_FILE\") | restic --repo {} --json backup {tags} --stdin --stdin-filename database.sql\n",
        shell_quote(&context.repository)
    ));
    script.push_str("DUMP_RC=$(cat \"$STATUS_FILE\")\n");
    script.push_str("if [ \"${DUMP_RC:-1}\" -ne 0 ]; then printf '数据库转储失败，退出码 %s\\n' \"$DUMP_RC\" >&2; exit \"$DUMP_RC\"; fi\n");
    let mut script_bytes = script.into_bytes();
    let result = execute_long_with_stdin(&connection.session, "sh -s", &script_bytes);
    utils::zeroize_bytes(&mut script_bytes);
    let output = result?;
    if output.exit_code != 0 {
        return Err(AppError::Message(sanitize(
            &display_output(&output),
            &secrets,
        )));
    }
    Ok(output)
}

fn list_snapshots(
    connection: &SshConnection,
    policy: &DataProtectionPolicy,
    context: &ResticContext,
) -> AppResult<Vec<ResticSnapshot>> {
    let output = context.run(
        &connection.session,
        &format!(
            "snapshots --tag {}",
            shell_quote(&format!("policy:{}", policy.id))
        ),
        true,
    )?;
    if output.stdout.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&output.stdout)
        .map_err(|error| AppError::Message(format!("无法解析 Restic 快照列表：{error}")))
}

fn connect(
    db: &Database,
    server_id: &str,
    known_hosts: &Path,
    allow_unknown_host: bool,
) -> AppResult<SshConnection> {
    let job = runbook::connection_job(db, server_id)?;
    ssh::connect(
        &job.server,
        &job.credentials,
        &job.jumps,
        known_hosts,
        allow_unknown_host,
    )
}

struct ResticContext {
    repository: String,
    secrets: Vec<String>,
    export_lines: String,
}

impl ResticContext {
    fn load(policy: &DataProtectionPolicy) -> AppResult<Self> {
        let password = get_secret(&policy.id, "repository-password")?.ok_or_else(|| {
            AppError::Message("Keychain 中没有 Restic 仓库密码，请编辑策略".into())
        })?;
        let mut secrets = vec![password.clone()];
        let mut export_lines = format!("export RESTIC_PASSWORD={}\n", shell_quote(&password));
        if policy.repository_kind == ResticRepositoryKind::S3 {
            let access = get_secret(&policy.id, "s3-access-key")?
                .ok_or_else(|| AppError::Message("Keychain 中没有 S3 Access Key".into()))?;
            let secret = get_secret(&policy.id, "s3-secret-key")?
                .ok_or_else(|| AppError::Message("Keychain 中没有 S3 Secret Key".into()))?;
            export_lines.push_str(&format!(
                "export AWS_ACCESS_KEY_ID={}\nexport AWS_SECRET_ACCESS_KEY={}\n",
                shell_quote(&access),
                shell_quote(&secret)
            ));
            secrets.extend([access, secret]);
        }
        Ok(Self {
            repository: policy.repository.clone(),
            secrets,
            export_lines,
        })
    }

    fn run(
        &self,
        session: &ssh2::Session,
        arguments: &str,
        json: bool,
    ) -> AppResult<ssh::ExecOutput> {
        let json = if json { " --json" } else { "" };
        // Pass secrets strictly via stdin to `sh -s` so they never appear in remote `ps aux` / `/proc/*/cmdline`.
        let script = format!(
            "{}exec restic --repo {}{} {}\n",
            self.export_lines,
            shell_quote(&self.repository),
            json,
            arguments
        );
        let mut script_bytes = script.into_bytes();
        let result = execute_long_with_stdin(session, "sh -s", &script_bytes);
        utils::zeroize_bytes(&mut script_bytes);
        let output = result?;
        if output.exit_code != 0 {
            return Err(AppError::Message(sanitize(
                &display_output(&output),
                &self.secrets,
            )));
        }
        Ok(output)
    }
}

fn execute_long_with_stdin(
    session: &ssh2::Session,
    command: &str,
    stdin_payload: &[u8],
) -> AppResult<ssh::ExecOutput> {
    match ssh::execute_with_stdin(
        session,
        command,
        stdin_payload,
        Duration::from_secs(86_400),
        &AtomicBool::new(false),
    )? {
        ssh::ControlledExecOutput::Completed(output) => Ok(output),
        ssh::ControlledExecOutput::TimedOut { .. } => {
            Err(AppError::Message("远程数据保护命令超过 24 小时".into()))
        }
        ssh::ControlledExecOutput::Cancelled { .. } => {
            Err(AppError::Message("远程数据保护命令已取消".into()))
        }
    }
}

#[derive(Deserialize)]
struct ResticMessage {
    message_type: Option<String>,
    snapshot_id: Option<String>,
}

fn parse_snapshot_id(output: &str) -> Option<String> {
    output.lines().rev().find_map(|line| {
        serde_json::from_str::<ResticMessage>(line)
            .ok()
            .and_then(|message| {
                (message.message_type.as_deref() == Some("summary"))
                    .then_some(message.snapshot_id)
                    .flatten()
            })
    })
}

fn execute_long(session: &ssh2::Session, command: &str) -> AppResult<ssh::ExecOutput> {
    match ssh::execute_controlled(
        session,
        command,
        Duration::from_secs(86_400),
        &AtomicBool::new(false),
    )? {
        ssh::ControlledExecOutput::Completed(output) => Ok(output),
        ssh::ControlledExecOutput::TimedOut { .. } => {
            Err(AppError::Message("远程数据保护命令超过 24 小时".into()))
        }
        ssh::ControlledExecOutput::Cancelled { .. } => {
            Err(AppError::Message("远程数据保护命令已取消".into()))
        }
    }
}

fn run_checked(
    connection: &SshConnection,
    command: &str,
    secrets: &[String],
) -> AppResult<ssh::ExecOutput> {
    let output = execute_long(&connection.session, command)?;
    if output.exit_code != 0 {
        return Err(AppError::Message(sanitize(
            &display_output(&output),
            secrets,
        )));
    }
    Ok(output)
}

fn display_output(output: &ssh::ExecOutput) -> String {
    let mut text = String::new();
    if !output.stdout.trim().is_empty() {
        text.push_str(output.stdout.trim());
    }
    if !output.stderr.trim().is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(output.stderr.trim());
    }
    if text.is_empty() {
        text.push_str("命令执行成功，没有输出");
    }
    text.chars().take(262_144).collect()
}

fn sanitize(value: &str, secrets: &[String]) -> String {
    let mut output = value.to_string();
    for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
        output = output.replace(secret, "[已隐藏]");
    }
    output
}

fn source_name(kind: ProtectionSourceKind) -> &'static str {
    match kind {
        ProtectionSourceKind::Directory => "directory",
        ProtectionSourceKind::DockerVolume => "docker_volume",
        ProtectionSourceKind::Postgresql => "postgresql",
        ProtectionSourceKind::Mysql => "mysql",
    }
}

fn save_secrets(id: &str, input: &DataProtectionInput) -> AppResult<()> {
    save_secret(
        id,
        "repository-password",
        input.repository_password.as_deref(),
    )?;
    save_secret(id, "s3-access-key", input.s3_access_key_id.as_deref())?;
    save_secret(id, "s3-secret-key", input.s3_secret_access_key.as_deref())?;
    save_secret(id, "database-password", input.database_password.as_deref())?;
    Ok(())
}

fn save_secret(id: &str, kind: &str, value: Option<&str>) -> AppResult<()> {
    match value {
        Some(value) if !value.is_empty() => ssh::credential_entry(id, kind)?
            .set_password(value)
            .map_err(|error| AppError::Keyring(error.to_string())),
        Some(_) => Ok(()),
        None => Ok(()),
    }
}

fn get_secret(id: &str, kind: &str) -> AppResult<Option<String>> {
    match ssh::credential_entry(id, kind)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(AppError::Keyring(error.to_string())),
    }
}

fn delete_secret(id: &str, kind: &str) -> AppResult<()> {
    match ssh::credential_entry(id, kind)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(AppError::Keyring(error.to_string())),
    }
}

fn delete_policy_secrets(id: &str) {
    for kind in [
        "repository-password",
        "s3-access-key",
        "s3-secret-key",
        "database-password",
    ] {
        let _ = delete_secret(id, kind);
    }
}
