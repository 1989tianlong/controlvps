use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::path::Path;
use std::sync::mpsc::{self, Sender, TryRecvError};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::{Server, TerminalSession};
use crate::ssh::{connect, Credentials, JumpHost};

pub enum TerminalCommand {
    Input(Vec<u8>),
    Resize(u32, u32),
    Close,
}

#[derive(Default)]
pub struct TerminalRegistry {
    senders: Mutex<HashMap<String, Sender<TerminalCommand>>>,
}

impl TerminalRegistry {
    pub fn send(&self, id: &str, command: TerminalCommand) -> AppResult<()> {
        let sender = self
            .senders
            .lock()
            .map_err(|_| AppError::Message("终端注册表不可用".into()))?
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::Message("终端会话已关闭".into()))?;
        sender
            .send(command)
            .map_err(|_| AppError::Message("无法写入终端会话".into()))
    }

    fn insert(&self, id: String, sender: Sender<TerminalCommand>) -> AppResult<()> {
        self.senders
            .lock()
            .map_err(|_| AppError::Message("终端注册表不可用".into()))?
            .insert(id, sender);
        Ok(())
    }

    fn remove(&self, id: &str) {
        if let Ok(mut senders) = self.senders.lock() {
            senders.remove(id);
        }
    }
}

pub fn open(
    app: AppHandle,
    registry: &TerminalRegistry,
    server: &Server,
    credentials: &Credentials,
    jump_host: Option<&JumpHost>,
    known_hosts_path: &Path,
    allow_unknown_host: bool,
) -> AppResult<TerminalSession> {
    let connection = connect(
        server,
        credentials,
        jump_host,
        known_hosts_path,
        allow_unknown_host,
    )?;
    let mut channel = connection.session.channel_session()?;
    channel.request_pty("xterm-256color", None, Some((100, 32, 0, 0)))?;
    channel.shell()?;
    connection.session.set_blocking(false);
    let terminal_id = Uuid::new_v4().to_string();
    let (sender, receiver) = mpsc::channel::<TerminalCommand>();
    registry.insert(terminal_id.clone(), sender)?;
    let thread_id = terminal_id.clone();
    let thread_app = app.clone();
    let fingerprint = connection.fingerprint.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        let mut buffer = [0_u8; 16 * 1024];
        let mut running = true;
        let mut exit_reason = "终端会话已关闭".to_string();
        let mut last_keepalive = Instant::now();
        while running {
            if last_keepalive.elapsed() >= Duration::from_secs(5) {
                let _ = connection.send_keepalive();
                last_keepalive = Instant::now();
            }
            loop {
                match receiver.try_recv() {
                    Ok(TerminalCommand::Input(data)) => {
                        if let Err(error) = channel.write_all(&data) {
                            exit_reason = format!("写入终端失败：{error}");
                            running = false;
                        }
                    }
                    Ok(TerminalCommand::Resize(columns, rows)) => {
                        let _ = channel.request_pty_size(columns, rows, None, None);
                    }
                    Ok(TerminalCommand::Close) => {
                        running = false;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        running = false;
                        break;
                    }
                }
            }
            if !running {
                break;
            }
            match channel.read(&mut buffer) {
                Ok(0) => {
                    std::thread::sleep(Duration::from_millis(8));
                }
                Ok(count) => {
                    let data = String::from_utf8_lossy(&buffer[..count]).to_string();
                    let _ = thread_app.emit(&format!("terminal-output-{thread_id}"), data);
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(8));
                }
                Err(error) => {
                    exit_reason = format!("终端连接中断：{error}");
                    break;
                }
            }
            if channel.eof() {
                break;
            }
        }
        let _ = channel.close();
        let _ = thread_app.emit(&format!("terminal-exit-{thread_id}"), exit_reason);
        if let Some(registry) = thread_app.try_state::<TerminalRegistry>() {
            registry.remove(&thread_id);
        }
    });
    Ok(TerminalSession {
        terminal_id,
        fingerprint,
    })
}
