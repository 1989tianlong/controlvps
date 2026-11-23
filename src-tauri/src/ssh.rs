use std::io::Read;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Instant;

use base64::Engine;
use keyring::Entry;
use sha2::{Digest, Sha256};
use ssh2::{CheckResult, KnownHostFileKind, Session};

use crate::error::{AppError, AppResult};
use crate::models::{AuthMethod, ConnectionTest, Server};

const KEYCHAIN_SERVICE: &str = "com.controlvps.app";

#[derive(Debug, Default)]
pub struct Credentials {
    pub password: Option<String>,
    pub key_passphrase: Option<String>,
}

pub struct SshConnection {
    pub session: Session,
    pub fingerprint: String,
}

pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

pub fn credential_entry(server_id: &str, kind: &str) -> AppResult<Entry> {
    Entry::new(KEYCHAIN_SERVICE, &format!("{server_id}:{kind}"))
        .map_err(|error| AppError::Keyring(error.to_string()))
}

pub fn save_credentials(server: &Server, input: &crate::models::ServerInput) -> AppResult<()> {
    match input.auth_method {
        AuthMethod::Password => {
            if let Some(password) = input.password.as_deref() {
                set_secret(&server.id, "password", password)?;
            }
            delete_secret(&server.id, "key-passphrase")?;
        }
        AuthMethod::Key => {
            if let Some(passphrase) = input.key_passphrase.as_deref() {
                set_secret(&server.id, "key-passphrase", passphrase)?;
            }
            delete_secret(&server.id, "password")?;
        }
        AuthMethod::Agent => {
            delete_secret(&server.id, "password")?;
            delete_secret(&server.id, "key-passphrase")?;
        }
    }
    Ok(())
}

pub fn delete_credentials(server_id: &str) -> AppResult<()> {
    delete_secret(server_id, "password")?;
    delete_secret(server_id, "key-passphrase")?;
    Ok(())
}

pub fn load_credentials(server: &Server) -> AppResult<Credentials> {
    Ok(Credentials {
        password: match server.auth_method {
            AuthMethod::Password => get_secret(&server.id, "password")?,
            _ => None,
        },
        key_passphrase: match server.auth_method {
            AuthMethod::Key => get_secret(&server.id, "key-passphrase")?,
            _ => None,
        },
    })
}

fn set_secret(server_id: &str, kind: &str, value: &str) -> AppResult<()> {
    let entry = credential_entry(server_id, kind)?;
    if value.is_empty() {
        delete_secret(server_id, kind)
    } else {
        entry
            .set_password(value)
            .map_err(|error| AppError::Keyring(error.to_string()))
    }
}

fn get_secret(server_id: &str, kind: &str) -> AppResult<Option<String>> {
    match credential_entry(server_id, kind)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(AppError::Keyring(error.to_string())),
    }
}

fn delete_secret(server_id: &str, kind: &str) -> AppResult<()> {
    match credential_entry(server_id, kind)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(AppError::Keyring(error.to_string())),
    }
}

pub fn connect(
    server: &Server,
    credentials: &Credentials,
    known_hosts_path: &Path,
    allow_unknown_host: bool,
) -> AppResult<SshConnection> {
    let address = (server.host.as_str(), server.port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| AppError::Message("无法解析服务器地址".into()))?;
    let stream = TcpStream::connect_timeout(&address, std::time::Duration::from_secs(10))?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(20)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(20)))?;

    let mut session = Session::new()?;
    session.set_tcp_stream(stream);
    session.set_timeout(20_000);
    session.handshake()?;
    let fingerprint = verify_host_key(&session, server, known_hosts_path, allow_unknown_host)?;

    match server.auth_method {
        AuthMethod::Password => {
            let password = credentials.password.as_deref().ok_or_else(|| {
                AppError::Message("钥匙串中没有密码，请编辑 VPS 并重新保存".into())
            })?;
            session.userauth_password(&server.username, password)?;
        }
        AuthMethod::Key => {
            let path = server
                .private_key_path
                .as_deref()
                .ok_or_else(|| AppError::Message("请选择私钥文件".into()))?;
            let path = expand_home(path)?;
            session.userauth_pubkey_file(
                &server.username,
                None,
                &path,
                credentials.key_passphrase.as_deref(),
            )?;
        }
        AuthMethod::Agent => {
            let mut agent = session.agent()?;
            agent.connect()?;
            agent.list_identities()?;
            let identities = agent.identities()?;
            if identities.is_empty() {
                return Err(AppError::Message("SSH Agent 中没有可用身份".into()));
            }
            for identity in identities {
                if agent.userauth(&server.username, &identity).is_ok() {
                    break;
                }
            }
        }
    }

    if !session.authenticated() {
        return Err(AppError::Message(
            "SSH 认证失败，请检查用户名和认证方式".into(),
        ));
    }
    Ok(SshConnection {
        session,
        fingerprint,
    })
}

fn verify_host_key(
    session: &Session,
    server: &Server,
    path: &Path,
    allow_unknown_host: bool,
) -> AppResult<String> {
    let (key, key_type) = session
        .host_key()
        .ok_or_else(|| AppError::Message("服务器没有提供主机密钥".into()))?;
    let fingerprint = format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(Sha256::digest(key))
    );
    let mut hosts = session.known_hosts()?;
    if path.exists() {
        hosts.read_file(path, KnownHostFileKind::OpenSSH)?;
    }
    match hosts.check_port(&server.host, server.port, key) {
        CheckResult::Match => Ok(fingerprint),
        CheckResult::Mismatch => Err(AppError::ChangedHostKey { fingerprint }),
        CheckResult::NotFound if allow_unknown_host => {
            let known_host = if server.port == 22 {
                server.host.clone()
            } else {
                format!("[{}]:{}", server.host, server.port)
            };
            hosts.add(&known_host, key, "ControlVPS", key_type.into())?;
            hosts.write_file(path, KnownHostFileKind::OpenSSH)?;
            Ok(fingerprint)
        }
        CheckResult::NotFound => Err(AppError::UnknownHostKey { fingerprint }),
        CheckResult::Failure => Err(AppError::Message("无法验证服务器主机密钥".into())),
    }
}

fn expand_home(path: &str) -> AppResult<PathBuf> {
    if path == "~" || path.starts_with("~/") {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| AppError::Message("无法解析 HOME 目录".into()))?;
        Ok(PathBuf::from(home).join(path.strip_prefix("~/").unwrap_or("")))
    } else {
        Ok(PathBuf::from(path))
    }
}

pub fn execute(session: &Session, command: &str) -> AppResult<ExecOutput> {
    let mut channel = session.channel_session()?;
    channel.exec(command)?;
    let mut stdout = String::new();
    let mut stderr = String::new();
    channel.read_to_string(&mut stdout)?;
    channel.stderr().read_to_string(&mut stderr)?;
    channel.wait_close()?;
    Ok(ExecOutput {
        stdout,
        stderr,
        exit_code: channel.exit_status()?,
    })
}

pub fn test(
    _server: &Server,
    connection: &SshConnection,
    started: Instant,
) -> AppResult<ConnectionTest> {
    let result = execute(
        &connection.session,
        "printf 'hostname='; hostname; if [ -r /etc/os-release ]; then . /etc/os-release; printf 'os=%s\\n' \"$PRETTY_NAME\"; else printf 'os='; uname -sr; fi",
    )?;
    let mut hostname = String::new();
    let mut os = String::new();
    for line in result.stdout.lines() {
        if let Some(value) = line.strip_prefix("hostname=") {
            hostname = value.to_string();
        } else if let Some(value) = line.strip_prefix("os=") {
            os = value.to_string();
        }
    }
    Ok(ConnectionTest {
        success: true,
        message: "SSH 连接与认证成功".into(),
        latency_ms: started.elapsed().as_millis() as u64,
        hostname,
        os,
        fingerprint: connection.fingerprint.clone(),
    })
}
