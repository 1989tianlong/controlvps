use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use base64::Engine;
use keyring::Entry;
use sha2::{Digest, Sha256};
use ssh2::{Channel, CheckResult, KnownHostFileKind, Session};

use crate::error::{AppError, AppResult};
use crate::models::{AuthMethod, ConnectionReport, ConnectionTest, DiagnosticStep, Server};

const KEYCHAIN_SERVICE: &str = "com.controlvps.app";
const KEEPALIVE_INTERVAL_SECONDS: u32 = 30;

#[derive(Debug, Default)]
pub struct Credentials {
    pub password: Option<String>,
    pub key_passphrase: Option<String>,
}

#[derive(Debug)]
pub struct JumpHost {
    pub server: Server,
    pub credentials: Credentials,
}

#[derive(Debug)]
pub struct ConnectionJob {
    pub server: Server,
    pub credentials: Credentials,
    pub jump_host: Option<JumpHost>,
}

pub struct SshConnection {
    pub session: Session,
    pub fingerprint: String,
    accepted_unknown_host: bool,
    jump_connection: Option<Box<SshConnection>>,
}

impl SshConnection {
    pub fn send_keepalive(&self) -> AppResult<()> {
        self.session.keepalive_send()?;
        if let Some(jump) = &self.jump_connection {
            jump.send_keepalive()?;
        }
        Ok(())
    }

    pub fn disconnect(&self) {
        let _ = self
            .session
            .disconnect(None, "ControlVPS connection closed", None);
        if let Some(jump) = &self.jump_connection {
            jump.disconnect();
        }
    }
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

#[derive(Default)]
struct ConnectionTrace {
    steps: Vec<DiagnosticStep>,
}

impl ConnectionTrace {
    fn step<T>(
        &mut self,
        name: impl Into<String>,
        operation: impl FnOnce() -> AppResult<(T, String)>,
    ) -> AppResult<T> {
        let name = name.into();
        let started = Instant::now();
        match operation() {
            Ok((value, detail)) => {
                self.steps.push(DiagnosticStep {
                    name,
                    success: true,
                    detail,
                    duration_ms: started.elapsed().as_millis() as u64,
                });
                Ok(value)
            }
            Err(error) => {
                self.steps.push(DiagnosticStep {
                    name,
                    success: false,
                    detail: error.to_string(),
                    duration_ms: started.elapsed().as_millis() as u64,
                });
                Err(error)
            }
        }
    }
}

pub fn connect(
    server: &Server,
    credentials: &Credentials,
    jump_host: Option<&JumpHost>,
    known_hosts_path: &Path,
    allow_unknown_host: bool,
) -> AppResult<SshConnection> {
    connect_inner(
        server,
        credentials,
        jump_host,
        known_hosts_path,
        allow_unknown_host,
        None,
    )
}

fn connect_inner(
    server: &Server,
    credentials: &Credentials,
    jump_host: Option<&JumpHost>,
    known_hosts_path: &Path,
    allow_unknown_host: bool,
    mut trace: Option<&mut ConnectionTrace>,
) -> AppResult<SshConnection> {
    if let Some(jump) = jump_host {
        if jump.server.id == server.id {
            return Err(AppError::Message("服务器不能使用自身作为跳板机".into()));
        }
        let jump_connection = connect_direct(
            &jump.server,
            &jump.credentials,
            known_hosts_path,
            allow_unknown_host,
            "跳板机",
            trace.as_deref_mut(),
        )?;
        let channel = match trace.as_deref_mut() {
            Some(trace) => trace.step("跳板通道", || {
                let channel = jump_connection.session.channel_direct_tcpip(
                    &server.host,
                    server.port,
                    None,
                )?;
                Ok((
                    channel,
                    format!("跳板机已转发到 {}:{}", server.host, server.port),
                ))
            })?,
            None => {
                jump_connection
                    .session
                    .channel_direct_tcpip(&server.host, server.port, None)?
            }
        };
        jump_connection.session.set_blocking(false);
        let stream = bridge_channel(channel)?;
        let target_allow_unknown = allow_unknown_host && !jump_connection.accepted_unknown_host;
        let mut connection = connect_on_stream(
            stream,
            server,
            credentials,
            known_hosts_path,
            target_allow_unknown,
            "目标主机",
            trace,
        )?;
        connection.jump_connection = Some(Box::new(jump_connection));
        Ok(connection)
    } else {
        let stream = open_tcp_stream(server, "目标主机", trace.as_deref_mut())?;
        connect_on_stream(
            stream,
            server,
            credentials,
            known_hosts_path,
            allow_unknown_host,
            "目标主机",
            trace,
        )
    }
}

fn connect_direct(
    server: &Server,
    credentials: &Credentials,
    known_hosts_path: &Path,
    allow_unknown_host: bool,
    label: &str,
    mut trace: Option<&mut ConnectionTrace>,
) -> AppResult<SshConnection> {
    let stream = open_tcp_stream(server, label, trace.as_deref_mut())?;
    connect_on_stream(
        stream,
        server,
        credentials,
        known_hosts_path,
        allow_unknown_host,
        label,
        trace,
    )
}

fn open_tcp_stream(
    server: &Server,
    label: &str,
    mut trace: Option<&mut ConnectionTrace>,
) -> AppResult<TcpStream> {
    let resolve = || -> AppResult<(std::net::SocketAddr, String)> {
        let address = (server.host.as_str(), server.port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| AppError::Message("无法解析服务器地址".into()))?;
        Ok((address, address.to_string()))
    };
    let address = match trace.as_deref_mut() {
        Some(trace) => trace.step(format!("{label} · 地址解析"), resolve)?,
        None => resolve()?.0,
    };
    let connect = || -> AppResult<(TcpStream, String)> {
        let stream = TcpStream::connect_timeout(&address, Duration::from_secs(10))?;
        stream.set_read_timeout(Some(Duration::from_secs(20)))?;
        stream.set_write_timeout(Some(Duration::from_secs(20)))?;
        stream.set_nodelay(true)?;
        Ok((stream, format!("已连接 {address}")))
    };
    match trace {
        Some(trace) => trace.step(format!("{label} · TCP 连接"), connect),
        None => connect().map(|result| result.0),
    }
}

fn connect_on_stream(
    stream: TcpStream,
    server: &Server,
    credentials: &Credentials,
    known_hosts_path: &Path,
    allow_unknown_host: bool,
    label: &str,
    mut trace: Option<&mut ConnectionTrace>,
) -> AppResult<SshConnection> {
    let handshake = || -> AppResult<(Session, String)> {
        let mut session = Session::new()?;
        session.set_tcp_stream(stream);
        session.set_timeout(20_000);
        session.handshake()?;
        Ok((session, "SSH 协议协商完成".into()))
    };
    let session = match trace.as_deref_mut() {
        Some(trace) => trace.step(format!("{label} · SSH 握手"), handshake)?,
        None => handshake()?.0,
    };
    let verify = || -> AppResult<((String, bool), String)> {
        let (fingerprint, accepted_unknown_host) =
            verify_host_key(&session, server, known_hosts_path, allow_unknown_host)?;
        Ok(((fingerprint.clone(), accepted_unknown_host), fingerprint))
    };
    let (fingerprint, accepted_unknown_host) = match trace.as_deref_mut() {
        Some(trace) => trace.step(format!("{label} · 主机密钥"), verify)?,
        None => verify()?.0,
    };
    let authenticate = || -> AppResult<((), String)> {
        authenticate(&session, server, credentials)?;
        Ok(((), format!("{} 认证通过", auth_label(&server.auth_method))))
    };
    match trace.as_deref_mut() {
        Some(trace) => trace.step(format!("{label} · 身份认证"), authenticate)?,
        None => {
            authenticate()?;
        }
    }
    session.set_keepalive(true, KEEPALIVE_INTERVAL_SECONDS);
    Ok(SshConnection {
        session,
        fingerprint,
        accepted_unknown_host,
        jump_connection: None,
    })
}

fn authenticate(session: &Session, server: &Server, credentials: &Credentials) -> AppResult<()> {
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
        return Err(AppError::Message(format!(
            "用户 {} 认证失败，请检查用户名和认证方式",
            server.username
        )));
    }
    Ok(())
}

fn auth_label(method: &AuthMethod) -> &'static str {
    match method {
        AuthMethod::Password => "密码",
        AuthMethod::Key => "私钥",
        AuthMethod::Agent => "SSH Agent",
    }
}

fn bridge_channel(channel: Channel) -> AppResult<TcpStream> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let client = TcpStream::connect(listener.local_addr()?)?;
    let (stream, _) = listener.accept()?;
    client.set_nodelay(true)?;
    stream.set_nodelay(true)?;
    std::thread::spawn(move || relay_jump(channel, stream));
    Ok(client)
}

fn relay_jump(channel: Channel, stream: TcpStream) {
    let Ok(mut local_reader) = stream.try_clone() else {
        return;
    };
    let mut local_writer = stream;
    let mut remote_reader = channel.clone();
    let mut remote_writer = channel;
    std::thread::scope(|scope| {
        scope.spawn(|| {
            pump(&mut local_reader, &mut remote_writer);
        });
        scope.spawn(|| {
            pump(&mut remote_reader, &mut local_writer);
            let _ = local_writer.shutdown(Shutdown::Both);
        });
    });
}

fn pump<R: Read, W: Write>(reader: &mut R, writer: &mut W) {
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            Err(_) => break,
        };
        let mut written = 0;
        while written < count {
            match writer.write(&buffer[written..count]) {
                Ok(0) => return,
                Ok(count) => written += count,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(_) => return,
            }
        }
    }
}

fn verify_host_key(
    session: &Session,
    server: &Server,
    path: &Path,
    allow_unknown_host: bool,
) -> AppResult<(String, bool)> {
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
        CheckResult::Match => Ok((fingerprint, false)),
        CheckResult::Mismatch => Err(AppError::ChangedHostKey {
            fingerprint,
            host: format!("{}:{} ({})", server.host, server.port, server.name),
        }),
        CheckResult::NotFound if allow_unknown_host => {
            let known_host = if server.port == 22 {
                server.host.clone()
            } else {
                format!("[{}]:{}", server.host, server.port)
            };
            hosts.add(&known_host, key, "ControlVPS", key_type.into())?;
            hosts.write_file(path, KnownHostFileKind::OpenSSH)?;
            Ok((fingerprint, true))
        }
        CheckResult::NotFound => Err(AppError::UnknownHostKey {
            fingerprint,
            host: format!("{}:{} ({})", server.host, server.port, server.name),
        }),
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

pub fn diagnose(
    server: &Server,
    credentials: &Credentials,
    jump_host: Option<&JumpHost>,
    known_hosts_path: &Path,
    allow_unknown_host: bool,
) -> AppResult<ConnectionReport> {
    let started = Instant::now();
    let mut trace = ConnectionTrace::default();
    let mut route = Vec::new();
    if let Some(jump) = jump_host {
        route.push(format!(
            "跳板：{} ({}:{})",
            jump.server.name, jump.server.host, jump.server.port
        ));
    }
    route.push(format!(
        "目标：{} ({}:{})",
        server.name, server.host, server.port
    ));
    let connection = match connect_inner(
        server,
        credentials,
        jump_host,
        known_hosts_path,
        allow_unknown_host,
        Some(&mut trace),
    ) {
        Ok(connection) => connection,
        Err(error @ (AppError::UnknownHostKey { .. } | AppError::ChangedHostKey { .. })) => {
            return Err(error);
        }
        Err(error) => {
            return Ok(ConnectionReport {
                success: false,
                message: error.to_string(),
                latency_ms: started.elapsed().as_millis() as u64,
                route,
                steps: trace.steps,
                hostname: String::new(),
                os: String::new(),
                fingerprint: String::new(),
            });
        }
    };
    match trace.step("目标主机 · 远程命令", || {
        let result = test(server, &connection, started)?;
        Ok((result, "已读取 hostname 与操作系统信息".into()))
    }) {
        Ok(result) => Ok(ConnectionReport {
            success: true,
            message: "连接链路完整，远程命令执行成功".into(),
            latency_ms: started.elapsed().as_millis() as u64,
            route,
            steps: trace.steps,
            hostname: result.hostname,
            os: result.os,
            fingerprint: result.fingerprint,
        }),
        Err(error) => Ok(ConnectionReport {
            success: false,
            message: error.to_string(),
            latency_ms: started.elapsed().as_millis() as u64,
            route,
            steps: trace.steps,
            hostname: String::new(),
            os: String::new(),
            fingerprint: connection.fingerprint.clone(),
        }),
    }
}
