use std::collections::{HashMap, VecDeque};
use std::io::{ErrorKind, Read, Write};
use std::path::Path;
use std::sync::mpsc::{self, Sender, TryRecvError};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::{Server, TerminalSession};
use crate::recording::Recorder;
use crate::ssh::{connect, Credentials, JumpHost};
use crate::utils::Utf8StreamDecoder;

const MAX_PENDING_INPUT_BYTES: usize = 2 * 1024 * 1024;

pub enum TerminalCommand {
    Input(Vec<u8>),
    Resize(u32, u32),
    Record(Option<Box<Recorder>>),
    Close,
}

#[derive(Debug, Clone)]
pub struct TerminalSessionInfo {
    pub server_id: String,
    pub server_name: String,
    pub columns: u32,
    pub rows: u32,
}

#[derive(Default)]
pub struct TerminalRegistry {
    senders: Mutex<HashMap<String, Sender<TerminalCommand>>>,
    sessions: Mutex<HashMap<String, TerminalSessionInfo>>,
}

impl TerminalRegistry {
    pub fn send(&self, id: &str, command: TerminalCommand) -> AppResult<()> {
        if let TerminalCommand::Resize(columns, rows) = command {
            if let Ok(mut sessions) = self.sessions.lock() {
                if let Some(info) = sessions.get_mut(id) {
                    info.columns = columns.max(20);
                    info.rows = rows.max(5);
                }
            }
        }
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

    pub fn session(&self, id: &str) -> AppResult<TerminalSessionInfo> {
        self.sessions
            .lock()
            .map_err(|_| AppError::Message("终端注册表不可用".into()))?
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::Message("终端会话已关闭".into()))
    }

    pub fn close_by_server(&self, server_id: &str) {
        let ids: Vec<String> = self
            .sessions
            .lock()
            .map(|sessions| {
                sessions
                    .iter()
                    .filter(|(_, info)| info.server_id == server_id)
                    .map(|(id, _)| id.clone())
                    .collect()
            })
            .unwrap_or_default();
        for id in ids {
            let _ = self.send(&id, TerminalCommand::Close);
        }
    }

    pub fn close_all(&self) {
        let senders: Vec<Sender<TerminalCommand>> = self
            .senders
            .lock()
            .map(|map| map.values().cloned().collect())
            .unwrap_or_default();
        for sender in senders {
            let _ = sender.send(TerminalCommand::Close);
        }
    }

    fn insert(
        &self,
        id: String,
        sender: Sender<TerminalCommand>,
        info: TerminalSessionInfo,
    ) -> AppResult<()> {
        self.senders
            .lock()
            .map_err(|_| AppError::Message("终端注册表不可用".into()))?
            .insert(id.clone(), sender);
        self.sessions
            .lock()
            .map_err(|_| AppError::Message("终端注册表不可用".into()))?
            .insert(id, info);
        Ok(())
    }

    fn remove(&self, id: &str) {
        if let Ok(mut senders) = self.senders.lock() {
            senders.remove(id);
        }
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(id);
        }
    }
}

fn stop_recording<R: tauri::Runtime>(
    recorder: &mut Option<Recorder>,
    app: &AppHandle<R>,
    thread_id: &str,
    reason: &str,
) {
    if let Some(finished) = recorder.take() {
        let _ = finished.finish();
    }
    let _ = app.emit(
        &format!("terminal-recording-stopped-{thread_id}"),
        reason.to_string(),
    );
}

pub fn open<R: tauri::Runtime>(
    app: AppHandle<R>,
    registry: &TerminalRegistry,
    server: &Server,
    credentials: &Credentials,
    jumps: &[JumpHost],
    known_hosts_path: &Path,
    allow_unknown_host: bool,
) -> AppResult<TerminalSession> {
    let connection = connect(
        server,
        credentials,
        jumps,
        known_hosts_path,
        allow_unknown_host,
    )?;
    let mut channel = connection.session.channel_session()?;
    channel.request_pty("xterm-256color", None, Some((100, 32, 0, 0)))?;
    channel.shell()?;
    connection.session.set_blocking(false);
    let terminal_id = Uuid::new_v4().to_string();
    let (sender, receiver) = mpsc::channel::<TerminalCommand>();
    registry.insert(
        terminal_id.clone(),
        sender,
        TerminalSessionInfo {
            server_id: server.id.clone(),
            server_name: server.name.clone(),
            columns: 100,
            rows: 32,
        },
    )?;
    let thread_id = terminal_id.clone();
    let thread_app = app.clone();
    let fingerprint = connection.fingerprint.clone();
    std::thread::Builder::new()
        .name(format!("controlvps-term-{}", &terminal_id[..8]))
        .spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            let mut buffer = [0_u8; 16 * 1024];
            let mut pending_input: VecDeque<u8> = VecDeque::new();
            let mut utf8 = Utf8StreamDecoder::default();
            let mut running = true;
            let mut exit_reason = "终端会话已关闭".to_string();
            let mut last_keepalive = Instant::now();
            let mut recorder: Option<Recorder> = None;

            while running {
                let mut progressed = false;

                if last_keepalive.elapsed() >= Duration::from_secs(5) {
                    let _ = connection.send_keepalive();
                    last_keepalive = Instant::now();
                }

                loop {
                    match receiver.try_recv() {
                        Ok(TerminalCommand::Input(data)) => {
                            // Do NOT record raw keystrokes ("i" frames) to avoid leaking sudo/DB passwords
                            // into plaintext .cast files; playback only renders terminal output ("o").
                            if pending_input.len() + data.len() <= MAX_PENDING_INPUT_BYTES {
                                pending_input.extend(data);
                                progressed = true;
                            }
                        }
                        Ok(TerminalCommand::Resize(columns, rows)) => {
                            if let Some(recorder) = recorder.as_mut() {
                                recorder.resize(columns, rows);
                            }
                            let _ = channel.request_pty_size(columns, rows, None, None);
                            progressed = true;
                        }
                        Ok(TerminalCommand::Record(next)) => {
                            if let Some(finished) = recorder.take() {
                                let _ = finished.finish();
                            }
                            recorder = next.map(|boxed| *boxed);
                            progressed = true;
                        }
                        Ok(TerminalCommand::Close) => {
                            running = false;
                            break;
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

                // Drain queued input safely in non-blocking mode (handle WouldBlock without disconnecting).
                while !pending_input.is_empty() {
                    let slice = pending_input.make_contiguous();
                    match channel.write(slice) {
                        Ok(0) => {
                            exit_reason = "写入终端失败：通道已关闭".into();
                            running = false;
                            break;
                        }
                        Ok(written) => {
                            pending_input.drain(..written);
                            progressed = true;
                        }
                        Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                        Err(error) => {
                            exit_reason = format!("写入终端失败：{error}");
                            running = false;
                            break;
                        }
                    }
                }
                if !running {
                    break;
                }

                match channel.read(&mut buffer) {
                    Ok(0) => {}
                    Ok(count) => {
                        progressed = true;
                        let text = utf8.decode(&buffer[..count]);
                        if !text.is_empty() {
                            let limit = recorder
                                .as_mut()
                                .map(|item| item.write_text("o", &text))
                                .unwrap_or(false);
                            if limit {
                                stop_recording(
                                    &mut recorder,
                                    &thread_app,
                                    &thread_id,
                                    "录制达到 64 MB 上限，已自动保存",
                                );
                            }
                            let _ = thread_app.emit(&format!("terminal-output-{thread_id}"), text);
                        }
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                    Err(error) => {
                        exit_reason = format!("终端连接中断：{error}");
                        break;
                    }
                }
                if channel.eof() {
                    break;
                }
                if !progressed {
                    std::thread::sleep(Duration::from_millis(8));
                }
            }

            let remaining = utf8.flush();
            if !remaining.is_empty() {
                if let Some(item) = recorder.as_mut() {
                    let _ = item.write_text("o", &remaining);
                }
                let _ = thread_app.emit(&format!("terminal-output-{thread_id}"), remaining);
            }
            if let Some(finished) = recorder.take() {
                let _ = finished.finish();
            }
            let _ = channel.close();
            connection.disconnect();
            let _ = thread_app.emit(&format!("terminal-exit-{thread_id}"), exit_reason);
            if let Some(registry) = thread_app.try_state::<TerminalRegistry>() {
                registry.remove(&thread_id);
            }
        })?;
    Ok(TerminalSession {
        terminal_id,
        fingerprint,
    })
}
