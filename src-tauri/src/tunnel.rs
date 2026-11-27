use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;

use crate::error::{AppError, AppResult};
use crate::models::{PortForwardKind, PortForwardRule, Server, TunnelStatus};
use crate::ssh::{self, Credentials, JumpHost, SshConnection};

const MAX_TUNNEL_CONNECTIONS: usize = 128;
const MAX_RELAY_BUFFER: usize = 64 * 1024;

struct TunnelHandle {
    server_id: String,
    kind: PortForwardKind,
    local_endpoint: String,
    stop: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
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
        jumps: &[JumpHost],
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
            jumps,
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
        let handle = TunnelHandle {
            server_id: server.id.clone(),
            kind: rule.kind,
            local_endpoint,
            stop: Arc::new(AtomicBool::new(false)),
            finished: Arc::new(AtomicBool::new(false)),
            active_connections: Arc::new(AtomicUsize::new(0)),
            total_connections: Arc::new(AtomicU64::new(0)),
            started_at: Utc::now().to_rfc3339(),
        };
        let status = status(&rule.id, &handle);
        let stop = handle.stop.clone();
        let finished = handle.finished.clone();
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
        std::thread::Builder::new()
            .name(format!("controlvps-tunnel-{}", &rule.id[..rule.id.len().min(8)]))
            .spawn(move || {
                // Own `connection` exclusively inside this single thread so libssh2 is never
                // called concurrently from multiple threads.
                if let Some(listener) = local_listener {
                    run_local_loop(
                        listener,
                        connection,
                        &rule,
                        stop.clone(),
                        active_connections,
                        total_connections,
                    );
                } else if let Some(listener) = remote_listener {
                    run_remote_loop(
                        listener,
                        connection,
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
            })?;
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
        if handle.kind != PortForwardKind::Remote {
            let wake_addr = handle
                .local_endpoint
                .replace("0.0.0.0:", "127.0.0.1:")
                .replace("[::]:", "[::1]:");
            let _ = TcpStream::connect(&wake_addr);
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

    pub fn stop_all(&self) {
        let ids = match self.inner.lock() {
            Ok(handles) => handles.keys().cloned().collect::<Vec<_>>(),
            Err(_) => return,
        };
        for id in ids {
            let _ = self.stop(&id);
        }
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

struct ActiveRelay {
    stream: TcpStream,
    channel: ssh2::Channel,
    to_remote: Vec<u8>,
    to_local: Vec<u8>,
    local_eof: bool,
    remote_eof: bool,
}

impl ActiveRelay {
    fn new(stream: TcpStream, channel: ssh2::Channel) -> Self {
        Self {
            stream,
            channel,
            to_remote: Vec::new(),
            to_local: Vec::new(),
            local_eof: false,
            remote_eof: false,
        }
    }

    /// Returns `(still_alive, made_progress)`.
    fn tick(&mut self, scratch: &mut [u8]) -> (bool, bool) {
        let mut progressed = false;

        if !self.local_eof && self.to_remote.len() < MAX_RELAY_BUFFER {
            match self.stream.read(scratch) {
                Ok(0) => {
                    self.local_eof = true;
                    progressed = true;
                }
                Ok(count) => {
                    self.to_remote.extend_from_slice(&scratch[..count]);
                    progressed = true;
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(_) => return (false, progressed),
            }
        }

        if !self.to_remote.is_empty() {
            match self.channel.write(&self.to_remote) {
                Ok(0) => return (false, progressed),
                Ok(count) => {
                    self.to_remote.drain(..count);
                    progressed = true;
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(_) => return (false, progressed),
            }
        } else if self.local_eof {
            let _ = self.channel.send_eof();
            return (false, true);
        }

        if !self.remote_eof && self.to_local.len() < MAX_RELAY_BUFFER {
            match self.channel.read(scratch) {
                Ok(0) => {
                    if self.channel.eof() {
                        self.remote_eof = true;
                        progressed = true;
                    }
                }
                Ok(count) => {
                    self.to_local.extend_from_slice(&scratch[..count]);
                    progressed = true;
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(_) => return (false, progressed),
            }
        }

        if !self.to_local.is_empty() {
            match self.stream.write(&self.to_local) {
                Ok(0) => return (false, progressed),
                Ok(count) => {
                    self.to_local.drain(..count);
                    progressed = true;
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(_) => return (false, progressed),
            }
        } else if self.remote_eof {
            return (false, true);
        }

        (true, progressed)
    }

    fn close(mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
        let _ = self.channel.close();
    }
}

struct PendingOpen {
    stream: TcpStream,
    host: String,
    port: u16,
    socks5: bool,
    deadline: Instant,
}

fn run_local_loop(
    listener: TcpListener,
    connection: SshConnection,
    rule: &PortForwardRule,
    stop: Arc<AtomicBool>,
    active_connections: Arc<AtomicUsize>,
    total_connections: Arc<AtomicU64>,
) {
    let mut relays: Vec<ActiveRelay> = Vec::new();
    let mut pending: Vec<PendingOpen> = Vec::new();
    let (socks_tx, socks_rx): (Sender<PendingOpen>, Receiver<PendingOpen>) = mpsc::channel();
    let mut scratch = [0_u8; 16 * 1024];
    let mut last_keepalive = Instant::now();

    while !stop.load(Ordering::Acquire) {
        let mut progressed = false;

        if last_keepalive.elapsed() >= Duration::from_secs(5) {
            let _ = connection.send_keepalive();
            last_keepalive = Instant::now();
        }

        // Accept new local connections (bounded by MAX_TUNNEL_CONNECTIONS).
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    progressed = true;
                    if relays.len() + pending.len() >= MAX_TUNNEL_CONNECTIONS {
                        let _ = stream.shutdown(Shutdown::Both);
                        continue;
                    }
                    let _ = stream.set_nodelay(true);
                    let _ = stream.set_nonblocking(true);
                    total_connections.fetch_add(1, Ordering::Relaxed);
                    match rule.kind {
                        PortForwardKind::Local => {
                            pending.push(PendingOpen {
                                stream,
                                host: rule.destination_host.clone(),
                                port: rule.destination_port,
                                socks5: false,
                                deadline: Instant::now() + Duration::from_secs(20),
                            });
                        }
                        PortForwardKind::Dynamic => {
                            let tx = socks_tx.clone();
                            let _ = std::thread::Builder::new()
                                .name("controlvps-socks5".into())
                                .spawn(move || {
                                    let mut stream = stream;
                                    if let Some((host, port)) = socks5_connect(&mut stream) {
                                        let _ = tx.send(PendingOpen {
                                            stream,
                                            host,
                                            port,
                                            socks5: true,
                                            deadline: Instant::now() + Duration::from_secs(20),
                                        });
                                    } else {
                                        let _ = stream.shutdown(Shutdown::Both);
                                    }
                                });
                        }
                        PortForwardKind::Remote => {}
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(_) => {
                    stop.store(true, Ordering::Release);
                    break;
                }
            }
        }

        // Collect completed SOCKS5 handshakes.
        while let Ok(item) = socks_rx.try_recv() {
            if relays.len() + pending.len() < MAX_TUNNEL_CONNECTIONS {
                pending.push(item);
                progressed = true;
            } else {
                let _ = item.stream.shutdown(Shutdown::Both);
            }
        }

        // Progress non-blocking channel_direct_tcpip opens on this single SSH thread.
        let mut idx = 0;
        while idx < pending.len() {
            let now = Instant::now();
            if now >= pending[idx].deadline {
                let mut expired = pending.swap_remove(idx);
                if expired.socks5 {
                    write_all_retry(&mut expired.stream, &[5, 1, 0, 1, 0, 0, 0, 0, 0, 0]);
                }
                let _ = expired.stream.shutdown(Shutdown::Both);
                progressed = true;
                continue;
            }
            match connection.session.channel_direct_tcpip(
                &pending[idx].host,
                pending[idx].port,
                None,
            ) {
                Ok(channel) => {
                    let mut ready = pending.swap_remove(idx);
                    if ready.socks5
                        && !write_all_retry(&mut ready.stream, &[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
                    {
                        let _ = ready.stream.shutdown(Shutdown::Both);
                        continue;
                    }
                    relays.push(ActiveRelay::new(ready.stream, channel));
                    progressed = true;
                }
                Err(error) => {
                    let io_error = std::io::Error::from(error);
                    if io_error.kind() == ErrorKind::WouldBlock {
                        idx += 1;
                    } else {
                        let mut failed = pending.swap_remove(idx);
                        if failed.socks5 {
                            write_all_retry(&mut failed.stream, &[5, 1, 0, 1, 0, 0, 0, 0, 0, 0]);
                        }
                        let _ = failed.stream.shutdown(Shutdown::Both);
                        progressed = true;
                    }
                }
            }
        }

        // Pump all active relays on this single thread.
        let mut r = 0;
        while r < relays.len() {
            let (alive, did_work) = relays[r].tick(&mut scratch);
            if did_work {
                progressed = true;
            }
            if !alive {
                let closed = relays.swap_remove(r);
                closed.close();
            } else {
                r += 1;
            }
        }

        active_connections.store(relays.len() + pending.len(), Ordering::Relaxed);

        if !progressed {
            let sleep_ms = if relays.is_empty() && pending.is_empty() {
                30
            } else {
                4
            };
            std::thread::sleep(Duration::from_millis(sleep_ms));
        }
    }

    for relay in relays {
        relay.close();
    }
    for item in pending {
        let _ = item.stream.shutdown(Shutdown::Both);
    }
    active_connections.store(0, Ordering::Relaxed);
    connection.disconnect();
}

struct PendingRemote {
    id: u64,
    channel: ssh2::Channel,
}

fn run_remote_loop(
    mut listener: ssh2::Listener,
    connection: SshConnection,
    rule: &PortForwardRule,
    stop: Arc<AtomicBool>,
    active_connections: Arc<AtomicUsize>,
    total_connections: Arc<AtomicU64>,
) {
    let mut relays: Vec<ActiveRelay> = Vec::new();
    let mut pending_remote: HashMap<u64, PendingRemote> = HashMap::new();
    let mut next_id: u64 = 1;
    type DialResult = (u64, Option<TcpStream>);
    let (dial_tx, dial_rx): (Sender<DialResult>, Receiver<DialResult>) = mpsc::channel();
    let mut scratch = [0_u8; 16 * 1024];
    let mut last_keepalive = Instant::now();

    while !stop.load(Ordering::Acquire) {
        let mut progressed = false;

        if last_keepalive.elapsed() >= Duration::from_secs(5) {
            let _ = connection.send_keepalive();
            last_keepalive = Instant::now();
        }

        loop {
            match listener.accept() {
                Ok(mut channel) => {
                    progressed = true;
                    if relays.len() + pending_remote.len() >= MAX_TUNNEL_CONNECTIONS {
                        let _ = channel.close();
                        continue;
                    }
                    total_connections.fetch_add(1, Ordering::Relaxed);
                    let id = next_id;
                    next_id = next_id.wrapping_add(1);
                    pending_remote.insert(id, PendingRemote { id, channel });
                    let dest_host = rule.destination_host.clone();
                    let dest_port = rule.destination_port;
                    let tx = dial_tx.clone();
                    let _ = std::thread::Builder::new()
                        .name("controlvps-remote-dial".into())
                        .spawn(move || {
                            let stream = dial_local_destination(&dest_host, dest_port);
                            let _ = tx.send((id, stream));
                        });
                }
                Err(error) => {
                    let io_error = std::io::Error::from(error);
                    if io_error.kind() == ErrorKind::WouldBlock {
                        break;
                    } else {
                        stop.store(true, Ordering::Release);
                        break;
                    }
                }
            }
        }

        while let Ok((id, maybe_stream)) = dial_rx.try_recv() {
            if let Some(mut item) = pending_remote.remove(&id) {
                progressed = true;
                match maybe_stream {
                    Some(stream) => {
                        let _ = item.id;
                        relays.push(ActiveRelay::new(stream, item.channel));
                    }
                    None => {
                        let _ = item.channel.close();
                    }
                }
            }
        }

        let mut r = 0;
        while r < relays.len() {
            let (alive, did_work) = relays[r].tick(&mut scratch);
            if did_work {
                progressed = true;
            }
            if !alive {
                let closed = relays.swap_remove(r);
                closed.close();
            } else {
                r += 1;
            }
        }

        active_connections.store(relays.len() + pending_remote.len(), Ordering::Relaxed);

        if !progressed {
            let sleep_ms = if relays.is_empty() && pending_remote.is_empty() {
                30
            } else {
                4
            };
            std::thread::sleep(Duration::from_millis(sleep_ms));
        }
    }

    for relay in relays {
        relay.close();
    }
    for (_, mut pending) in pending_remote {
        let _ = pending.channel.close();
    }
    active_connections.store(0, Ordering::Relaxed);
    connection.disconnect();
}

fn dial_local_destination(destination_host: &str, destination_port: u16) -> Option<TcpStream> {
    let addresses = (destination_host, destination_port)
        .to_socket_addrs()
        .ok()?
        .take(6);
    for address in addresses {
        if let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_secs(10)) {
            let _ = stream.set_nodelay(true);
            let _ = stream.set_nonblocking(true);
            return Some(stream);
        }
    }
    None
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

fn read_exact_retry(stream: &mut TcpStream, mut buffer: &mut [u8]) -> bool {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !buffer.is_empty() {
        match stream.read(buffer) {
            Ok(0) => return false,
            Ok(count) => buffer = &mut buffer[count..],
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
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
    let deadline = Instant::now() + Duration::from_secs(20);
    while !buffer.is_empty() {
        match stream.write(buffer) {
            Ok(0) => return false,
            Ok(count) => buffer = &buffer[count..],
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return false,
        }
    }
    true
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
