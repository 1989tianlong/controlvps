use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;

use crate::error::{AppError, AppResult};
use crate::models::{PortForwardKind, PortForwardRule, Server, TunnelStatus};
use crate::ssh::{self, Credentials, JumpHost, SshConnection};

struct TunnelHandle {
    server_id: String,
    kind: PortForwardKind,
    local_endpoint: String,
    stop: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
    session: Arc<Mutex<SshConnection>>,
    active_connections: Arc<AtomicUsize>,
    total_connections: Arc<AtomicU64>,
    started_at: String,
}

#[derive(Clone, Default)]
pub struct TunnelRegistry {
    inner: Arc<Mutex<HashMap<String, TunnelHandle>>>,
}

impl TunnelRegistry {
    pub fn start(
        &self,
        rule: &PortForwardRule,
        server: &Server,
        credentials: &Credentials,
        jump_host: Option<&JumpHost>,
        known_hosts: &std::path::Path,
        allow_unknown_host: bool,
    ) -> AppResult<TunnelStatus> {
        if self
            .inner
            .lock()
            .map_err(|_| registry_error())?
            .contains_key(&rule.id)
        {
            return Err(AppError::Message("这条隧道已经在运行".into()));
        }
        let connection = ssh::connect(
            server,
            credentials,
            jump_host,
            known_hosts,
            allow_unknown_host,
        )?;
        let (local_endpoint, local_listener, remote_listener) = match rule.kind {
            PortForwardKind::Remote => {
                let (listener, bound_port) = connection
                    .session
                    .channel_forward_listen(
                        rule.listen_port,
                        Some(rule.listen_address.as_str()),
                        Some(16),
                    )
                    .map_err(|error| {
                        AppError::Message(format!(
                            "VPS 无法监听远程端口，请检查 AllowTcpForwarding、GatewayPorts 和端口占用：{error}"
                        ))
                    })?;
                (
                    format_endpoint(&rule.listen_address, bound_port),
                    None,
                    Some(listener),
                )
            }
            PortForwardKind::Local | PortForwardKind::Dynamic => {
                let listener = TcpListener::bind((rule.listen_address.as_str(), rule.listen_port))?;
                listener.set_nonblocking(true)?;
                (listener.local_addr()?.to_string(), Some(listener), None)
            }
        };
        connection.session.set_blocking(false);
        let session = Arc::new(Mutex::new(connection));
        let handle = TunnelHandle {
            server_id: server.id.clone(),
            kind: rule.kind,
            local_endpoint,
            stop: Arc::new(AtomicBool::new(false)),
            finished: Arc::new(AtomicBool::new(false)),
            session,
            active_connections: Arc::new(AtomicUsize::new(0)),
            total_connections: Arc::new(AtomicU64::new(0)),
            started_at: Utc::now().to_rfc3339(),
        };
        let status = status(&rule.id, &handle);
        let stop = handle.stop.clone();
        let finished = handle.finished.clone();
        let session = handle.session.clone();
        let active_connections = handle.active_connections.clone();
        let total_connections = handle.total_connections.clone();
        let mut registry = self.inner.lock().map_err(|_| registry_error())?;
        if registry.contains_key(&rule.id) {
            return Err(AppError::Message("这条隧道已经在运行".into()));
        }
        registry.insert(rule.id.clone(), handle);
        drop(registry);

        let registry = self.clone();
        let rule = rule.clone();
        std::thread::spawn(move || {
            if let Some(listener) = local_listener {
                run_listener(
                    listener,
                    session,
                    &rule,
                    stop.clone(),
                    active_connections,
                    total_connections,
                );
            } else if let Some(listener) = remote_listener {
                run_remote_listener(
                    listener,
                    session,
                    &rule,
                    stop.clone(),
                    active_connections,
                    total_connections,
                );
            }
            if let Ok(mut handles) = registry.inner.lock() {
                let same_tunnel = handles
                    .get(&rule.id)
                    .is_some_and(|handle| Arc::ptr_eq(&handle.stop, &stop));
                if same_tunnel {
                    handles.remove(&rule.id);
                }
            }
            finished.store(true, Ordering::Release);
        });
        Ok(status)
    }

    pub fn stop(&self, rule_id: &str) -> AppResult<()> {
        let handle = self
            .inner
            .lock()
            .map_err(|_| registry_error())?
            .remove(rule_id);
        let Some(handle) = handle else {
            return Ok(());
        };
        handle.stop.store(true, Ordering::Release);
        if let Ok(connection) = handle.session.lock() {
            connection.disconnect();
        }
        if handle.kind != PortForwardKind::Remote {
            let _ = TcpStream::connect(&handle.local_endpoint);
        }
        for _ in 0..200 {
            if handle.finished.load(Ordering::Acquire) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Err(AppError::Message("停止隧道超时，请重试".into()))
    }

    pub fn stop_by_server(&self, server_id: &str) -> AppResult<()> {
        let ids = self
            .inner
            .lock()
            .map_err(|_| registry_error())?
            .iter()
            .filter(|(_, handle)| handle.server_id == server_id)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            self.stop(&id)?;
        }
        Ok(())
    }

    pub fn list(&self) -> AppResult<Vec<TunnelStatus>> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| registry_error())?
            .iter()
            .map(|(id, handle)| status(id, handle))
            .collect())
    }
}

fn run_listener(
    listener: TcpListener,
    session: Arc<Mutex<SshConnection>>,
    rule: &PortForwardRule,
    stop: Arc<AtomicBool>,
    active_connections: Arc<AtomicUsize>,
    total_connections: Arc<AtomicU64>,
) {
    let mut last_keepalive = Instant::now();
    while !stop.load(Ordering::Acquire) {
        if last_keepalive.elapsed() >= Duration::from_secs(5) {
            if let Ok(connection) = session.lock() {
                let _ = connection.send_keepalive();
            }
            last_keepalive = Instant::now();
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nodelay(true);
                let _ = stream.set_nonblocking(true);
                total_connections.fetch_add(1, Ordering::Relaxed);
                active_connections.fetch_add(1, Ordering::Relaxed);
                let session = session.clone();
                let kind = rule.kind;
                let destination_host = rule.destination_host.clone();
                let destination_port = rule.destination_port;
                let active_connections = active_connections.clone();
                std::thread::spawn(move || {
                    match kind {
                        PortForwardKind::Local => {
                            handle_connection(stream, session, &destination_host, destination_port)
                        }
                        PortForwardKind::Dynamic => handle_socks5_connection(stream, session),
                        PortForwardKind::Remote => {}
                    }
                    active_connections.fetch_sub(1, Ordering::Relaxed);
                });
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => break,
        }
    }
}

fn run_remote_listener(
    mut listener: ssh2::Listener,
    session: Arc<Mutex<SshConnection>>,
    rule: &PortForwardRule,
    stop: Arc<AtomicBool>,
    active_connections: Arc<AtomicUsize>,
    total_connections: Arc<AtomicU64>,
) {
    let mut last_keepalive = Instant::now();
    while !stop.load(Ordering::Acquire) {
        if last_keepalive.elapsed() >= Duration::from_secs(5) {
            if let Ok(connection) = session.lock() {
                let _ = connection.send_keepalive();
            }
            last_keepalive = Instant::now();
        }
        match listener.accept() {
            Ok(channel) => {
                total_connections.fetch_add(1, Ordering::Relaxed);
                active_connections.fetch_add(1, Ordering::Relaxed);
                let destination_host = rule.destination_host.clone();
                let destination_port = rule.destination_port;
                let active_connections = active_connections.clone();
                std::thread::spawn(move || {
                    handle_remote_connection(channel, &destination_host, destination_port);
                    active_connections.fetch_sub(1, Ordering::Relaxed);
                });
            }
            Err(error) => {
                let error = std::io::Error::from(error);
                if error.kind() == ErrorKind::WouldBlock {
                    std::thread::sleep(Duration::from_millis(100));
                } else {
                    break;
                }
            }
        }
    }
}

fn handle_remote_connection(channel: ssh2::Channel, destination_host: &str, destination_port: u16) {
    let Some(address) = (destination_host, destination_port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut addresses| addresses.next())
    else {
        return;
    };
    let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_secs(10)) else {
        return;
    };
    let _ = stream.set_nodelay(true);
    let _ = stream.set_nonblocking(true);
    relay(stream, channel);
}

fn handle_connection(
    stream: TcpStream,
    session: Arc<Mutex<SshConnection>>,
    destination_host: &str,
    destination_port: u16,
) {
    let Some(channel) = open_channel(&session, destination_host, destination_port) else {
        return;
    };
    relay(stream, channel);
}

fn handle_socks5_connection(mut stream: TcpStream, session: Arc<Mutex<SshConnection>>) {
    let Some((destination_host, destination_port)) = socks5_connect(&mut stream) else {
        return;
    };
    let Some(channel) = open_channel(&session, &destination_host, destination_port) else {
        write_all_retry(&mut stream, &[5, 1, 0, 1, 0, 0, 0, 0, 0, 0]);
        return;
    };
    if !write_all_retry(&mut stream, &[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]) {
        return;
    }
    relay(stream, channel);
}

fn socks5_connect(stream: &mut TcpStream) -> Option<(String, u16)> {
    let mut greeting = [0_u8; 2];
    if !read_exact_retry(stream, &mut greeting) || greeting[0] != 5 || greeting[1] == 0 {
        return None;
    }
    let mut methods = vec![0_u8; greeting[1] as usize];
    if !read_exact_retry(stream, &mut methods) || !methods.contains(&0) {
        write_all_retry(stream, &[5, 0xff]);
        return None;
    }
    if !write_all_retry(stream, &[5, 0]) {
        return None;
    }

    let mut request = [0_u8; 4];
    if !read_exact_retry(stream, &mut request) || request[0] != 5 || request[2] != 0 {
        return None;
    }
    if request[1] != 1 {
        write_all_retry(stream, &[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]);
        return None;
    }
    let host = match request[3] {
        1 => {
            let mut address = [0_u8; 4];
            if !read_exact_retry(stream, &mut address) {
                return None;
            }
            std::net::Ipv4Addr::from(address).to_string()
        }
        3 => {
            let mut length = [0_u8; 1];
            if !read_exact_retry(stream, &mut length) || length[0] == 0 {
                return None;
            }
            let mut domain = vec![0_u8; length[0] as usize];
            if !read_exact_retry(stream, &mut domain) {
                return None;
            }
            String::from_utf8(domain).ok()?
        }
        4 => {
            let mut address = [0_u8; 16];
            if !read_exact_retry(stream, &mut address) {
                return None;
            }
            std::net::Ipv6Addr::from(address).to_string()
        }
        _ => {
            write_all_retry(stream, &[5, 8, 0, 1, 0, 0, 0, 0, 0, 0]);
            return None;
        }
    };
    let mut port = [0_u8; 2];
    if !read_exact_retry(stream, &mut port) {
        return None;
    }
    Some((host, u16::from_be_bytes(port)))
}

fn open_channel(
    session: &Arc<Mutex<SshConnection>>,
    destination_host: &str,
    destination_port: u16,
) -> Option<ssh2::Channel> {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let result = match session.lock() {
            Ok(connection) => {
                connection
                    .session
                    .channel_direct_tcpip(destination_host, destination_port, None)
            }
            Err(_) => return None,
        };
        match result {
            Ok(channel) => return Some(channel),
            Err(error) => {
                let error = std::io::Error::from(error);
                if error.kind() != ErrorKind::WouldBlock {
                    return None;
                }
                if std::time::Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

fn relay(stream: TcpStream, channel: ssh2::Channel) {
    let Ok(mut local_reader) = stream.try_clone() else {
        return;
    };
    let mut local_writer = stream;
    let mut remote_reader = channel.stream(0);
    let mut remote_writer = channel.stream(0);
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

fn read_exact_retry(stream: &mut TcpStream, mut buffer: &mut [u8]) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while !buffer.is_empty() {
        match stream.read(buffer) {
            Ok(0) => return false,
            Ok(count) => buffer = &mut buffer[count..],
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if std::time::Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return false,
        }
    }
    true
}

fn write_all_retry(stream: &mut TcpStream, mut buffer: &[u8]) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while !buffer.is_empty() {
        match stream.write(buffer) {
            Ok(0) => return false,
            Ok(count) => buffer = &buffer[count..],
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if std::time::Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return false,
        }
    }
    true
}

fn pump<R: Read, W: Write>(reader: &mut R, writer: &mut W) {
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
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
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => return,
            }
        }
    }
}

fn format_endpoint(address: &str, port: u16) -> String {
    if address.contains(':') {
        format!("[{address}]:{port}")
    } else {
        format!("{address}:{port}")
    }
}

fn status(id: &str, handle: &TunnelHandle) -> TunnelStatus {
    TunnelStatus {
        rule_id: id.to_string(),
        server_id: handle.server_id.clone(),
        local_endpoint: handle.local_endpoint.clone(),
        active_connections: handle.active_connections.load(Ordering::Relaxed),
        total_connections: handle.total_connections.load(Ordering::Relaxed),
        started_at: handle.started_at.clone(),
    }
}

fn registry_error() -> AppError {
    AppError::Message("隧道注册表不可用".into())
}
