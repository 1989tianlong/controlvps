use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use ssh2::Session;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::ssh::{self, ConnectionJob};
use crate::utils::{shell_quote, Utf8StreamDecoder};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessEntry {
    pub pid: u32,
    pub ppid: u32,
    pub user: String,
    pub cpu_percent: f64,
    pub mem_percent: f64,
    pub rss_kb: u64,
    pub stat: String,
    pub elapsed: String,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LogStreamSource {
    Systemd {
        unit: Option<String>,
        lines: Option<u32>,
    },
    Docker {
        container: String,
        lines: Option<u32>,
    },
    File {
        path: String,
        lines: Option<u32>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogStreamDataEvent {
    pub stream_id: String,
    pub chunk: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogStreamClosedEvent {
    pub stream_id: String,
    pub reason: String,
}

#[derive(Default, Clone)]
pub struct LogStreamRegistry {
    streams: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

impl LogStreamRegistry {
    pub fn insert(&self, id: String, cancel: Arc<AtomicBool>) {
        if let Ok(mut guard) = self.streams.lock() {
            guard.insert(id, cancel);
        }
    }

    pub fn stop(&self, id: &str) -> bool {
        if let Ok(mut guard) = self.streams.lock() {
            if let Some(flag) = guard.remove(id) {
                flag.store(true, Ordering::Relaxed);
                return true;
            }
        }
        false
    }

    pub fn remove(&self, id: &str) {
        if let Ok(mut guard) = self.streams.lock() {
            guard.remove(id);
        }
    }

    pub fn stop_all(&self) {
        if let Ok(mut guard) = self.streams.lock() {
            for (_, flag) in guard.drain() {
                flag.store(true, Ordering::Relaxed);
            }
        }
    }
}

pub fn list_processes(session: &Session, limit: usize) -> AppResult<Vec<ProcessEntry>> {
    let limit = limit.clamp(10, 200);
    let command = format!(
        "LC_ALL=C ps -eo pid=,ppid=,user=,pcpu=,pmem=,rss=,stat=,etime=,args= --sort=-pcpu 2>/dev/null | head -n {limit} || LC_ALL=C ps -eo pid=,ppid=,user=,pcpu=,pmem=,rss=,stat=,etime=,args= | head -n {limit}"
    );
    let output = ssh::execute(session, &command)?;
    Ok(parse_ps_output(&output.stdout))
}

pub fn signal_process(session: &Session, pid: u32, signal: &str) -> AppResult<String> {
    if pid <= 1 {
        return Err(AppError::Message("禁止向 PID <= 1 的系统核心进程发送信号".into()));
    }
    let normalized = validate_signal(signal)?;
    let command = format!("kill -s {normalized} {pid} 2>&1");
    let output = ssh::execute(session, &command)?;
    if output.exit_code != 0 {
        let msg = format!("{}{}", output.stdout, output.stderr).trim().to_string();
        return Err(AppError::Message(if msg.is_empty() {
            format!("发送 SIG{normalized} 给 PID {pid} 失败（退出码 {}）", output.exit_code)
        } else {
            format!("发送 SIG{normalized} 失败：{msg}")
        }));
    }
    Ok(format!("已向 PID {pid} 发送 SIG{normalized}"))
}

pub fn start_log_stream(
    app: AppHandle,
    registry: LogStreamRegistry,
    job: ConnectionJob,
    known_hosts: PathBuf,
    allow_unknown_host: bool,
    source: LogStreamSource,
) -> AppResult<String> {
    let command = build_log_command(&source)?;
    let stream_id = Uuid::new_v4().to_string();
    let cancel = Arc::new(AtomicBool::new(false));
    registry.insert(stream_id.clone(), cancel.clone());

    let stream_id_clone = stream_id.clone();
    std::thread::spawn(move || {
        let reason = match run_stream_loop(
            &app,
            &stream_id_clone,
            &job,
            &known_hosts,
            allow_unknown_host,
            &command,
            &cancel,
        ) {
            Ok(msg) => msg,
            Err(err) => err.to_string(),
        };
        registry.remove(&stream_id_clone);
        let _ = app.emit(
            "log-stream-closed",
            LogStreamClosedEvent {
                stream_id: stream_id_clone,
                reason,
            },
        );
    });

    Ok(stream_id)
}

fn run_stream_loop(
    app: &AppHandle,
    stream_id: &str,
    job: &ConnectionJob,
    known_hosts: &std::path::Path,
    allow_unknown_host: bool,
    command: &str,
    cancel: &AtomicBool,
) -> AppResult<String> {
    let connection = ssh::connect(
        &job.server,
        &job.credentials,
        &job.jumps,
        known_hosts,
        allow_unknown_host,
    )?;
    let mut channel = connection.session.channel_session()?;
    channel.exec(command)?;
    connection.session.set_blocking(false);

    let mut decoder = Utf8StreamDecoder::default();
    let mut stderr_decoder = Utf8StreamDecoder::default();
    let mut buf = [0u8; 8192];
    let mut reason = "日志流已结束".to_string();

    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = channel.write_all(&[3]);
            let _ = channel.flush();
            let _ = channel.send_eof();
            let _ = channel.close();
            reason = "已停止日志流".to_string();
            break;
        }

        let mut progressed = false;
        match channel.read(&mut buf) {
            Ok(0) => {
                if channel.eof() {
                    break;
                }
            }
            Ok(n) => {
                progressed = true;
                let chunk = decoder.decode(&buf[..n]);
                if !chunk.is_empty() {
                    let _ = app.emit(
                        "log-stream-data",
                        LogStreamDataEvent {
                            stream_id: stream_id.to_string(),
                            chunk,
                        },
                    );
                }
            }
            Err(err) if err.kind() == ErrorKind::WouldBlock => {}
            Err(err) => {
                reason = format!("读取日志流出错：{err}");
                break;
            }
        }

        match channel.stderr().read(&mut buf) {
            Ok(n) if n > 0 => {
                progressed = true;
                let chunk = stderr_decoder.decode(&buf[..n]);
                if !chunk.is_empty() {
                    let _ = app.emit(
                        "log-stream-data",
                        LogStreamDataEvent {
                            stream_id: stream_id.to_string(),
                            chunk,
                        },
                    );
                }
            }
            Ok(_) => {}
            Err(err) if err.kind() == ErrorKind::WouldBlock => {}
            Err(_) => {}
        }

        if !progressed {
            if channel.eof() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    let tail = format!("{}{}", decoder.flush(), stderr_decoder.flush());
    if !tail.is_empty() {
        let _ = app.emit(
            "log-stream-data",
            LogStreamDataEvent {
                stream_id: stream_id.to_string(),
                chunk: tail,
            },
        );
    }
    connection.session.set_blocking(true);
    connection.disconnect();
    Ok(reason)
}

pub fn build_log_command(source: &LogStreamSource) -> AppResult<String> {
    match source {
        LogStreamSource::Systemd { unit, lines } => {
            let n = lines.unwrap_or(100).clamp(10, 1000);
            if let Some(u) = unit.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                if u.chars().any(|c| c.is_control() || c.is_whitespace()) {
                    return Err(AppError::Message("无效的 systemd 服务名".into()));
                }
                Ok(format!(
                    "journalctl -u {} -n {n} -f --no-pager 2>&1",
                    shell_quote(u)
                ))
            } else {
                Ok(format!("journalctl -n {n} -f --no-pager 2>&1"))
            }
        }
        LogStreamSource::Docker { container, lines } => {
            let name = container.trim();
            if name.is_empty() || name.chars().any(|c| c.is_control() || c.is_whitespace()) {
                return Err(AppError::Message("无效的容器名称或 ID".into()));
            }
            let n = lines.unwrap_or(100).clamp(10, 1000);
            Ok(format!("docker logs -f --tail {n} {} 2>&1", shell_quote(name)))
        }
        LogStreamSource::File { path, lines } => {
            let trimmed = path.trim();
            if trimmed.is_empty() || trimmed.chars().any(|c| c.is_control()) {
                return Err(AppError::Message("请输入有效的日志文件路径".into()));
            }
            let n = lines.unwrap_or(100).clamp(10, 1000);
            Ok(format!("tail -n {n} -F {} 2>&1", shell_quote(trimmed)))
        }
    }
}

pub fn validate_signal(signal: &str) -> AppResult<&'static str> {
    match signal.trim().to_ascii_uppercase().trim_start_matches("SIG") {
        "TERM" => Ok("TERM"),
        "KILL" => Ok("KILL"),
        "HUP" => Ok("HUP"),
        "INT" => Ok("INT"),
        _ => Err(AppError::Message(
            "仅支持 TERM / KILL / HUP / INT 信号".into(),
        )),
    }
}

pub fn parse_ps_output(raw: &str) -> Vec<ProcessEntry> {
    let mut entries = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(pid_s) = parts.next() else { continue };
        let Ok(pid) = pid_s.parse::<u32>() else { continue };
        let Some(ppid_s) = parts.next() else { continue };
        let ppid = ppid_s.parse::<u32>().unwrap_or(0);
        let Some(user) = parts.next() else { continue };
        let Some(cpu_s) = parts.next() else { continue };
        let Some(mem_s) = parts.next() else { continue };
        let Some(rss_s) = parts.next() else { continue };
        let Some(stat) = parts.next() else { continue };
        let Some(elapsed) = parts.next() else { continue };
        let command: String = parts.collect::<Vec<_>>().join(" ");
        if command.is_empty() {
            continue;
        }
        entries.push(ProcessEntry {
            pid,
            ppid,
            user: user.to_string(),
            cpu_percent: cpu_s.parse::<f64>().unwrap_or(0.0),
            mem_percent: mem_s.parse::<f64>().unwrap_or(0.0),
            rss_kb: rss_s.parse::<u64>().unwrap_or(0),
            stat: stat.to_string(),
            elapsed: elapsed.to_string(),
            command,
        });
    }
    entries
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthlyTrafficReport {
    pub interface: String,
    pub source: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub total_bytes: u64,
    pub quota_gb: Option<f64>,
    pub used_percent: Option<f64>,
    pub warning: bool,
}

pub fn collect_monthly_traffic(
    session: &Session,
    quota_gb: Option<f64>,
) -> AppResult<MonthlyTrafficReport> {
    let script = r#"
if command -v vnstat >/dev/null 2>&1; then
  VN=$(vnstat --json m 1 2>/dev/null || true)
  if [ -n "$VN" ]; then
    printf '__VNSTAT__\n%s\n' "$VN"
    exit 0
  fi
fi
IFACE=$(ip route show default 2>/dev/null | awk '{for(i=1;i<=NF;i++) if($i=="dev"){print $(i+1); exit}}')
if [ -z "$IFACE" ]; then
  IFACE=$(awk -F: 'NR>2 && $1 !~ /lo|docker|br-|veth|virbr|tailscale/ {gsub(/ /,"",$1); print $1; exit}' /proc/net/dev 2>/dev/null)
fi
if [ -n "$IFACE" ]; then
  LINE=$(awk -v dev="$IFACE" -F'[: ]+' '$1==dev || $2==dev {if($1=="") print $3, $11; else print $2, $10; exit}' /proc/net/dev 2>/dev/null)
  printf '__PROC__\n%s %s\n' "$IFACE" "$LINE"
fi
"#;
    let output = ssh::execute(session, script)?;
    Ok(parse_traffic_output(&output.stdout, quota_gb))
}

pub fn parse_traffic_output(raw: &str, quota_gb: Option<f64>) -> MonthlyTrafficReport {
    let mut iface = "eth0".to_string();
    let mut source = "proc_net_dev".to_string();
    let mut rx_bytes = 0u64;
    let mut tx_bytes = 0u64;

    if let Some(rest) = raw.strip_prefix("__VNSTAT__\n").or_else(|| raw.split("__VNSTAT__\n").nth(1)) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(rest.trim()) {
            if let Some(first) = json
                .get("interfaces")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
            {
                if let Some(name) = first.get("name").and_then(|v| v.as_str()) {
                    iface = name.to_string();
                }
                if let Some(month) = first
                    .get("traffic")
                    .and_then(|t| t.get("month"))
                    .and_then(|m| m.as_array())
                    .and_then(|arr| arr.last())
                {
                    rx_bytes = month.get("rx").and_then(|v| v.as_u64()).unwrap_or(0);
                    tx_bytes = month.get("tx").and_then(|v| v.as_u64()).unwrap_or(0);
                    source = "vnstat".to_string();
                }
            }
        }
    } else if let Some(rest) = raw.split("__PROC__\n").nth(1) {
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.len() >= 3 {
            iface = parts[0].to_string();
            rx_bytes = parts[1].parse::<u64>().unwrap_or(0);
            tx_bytes = parts[2].parse::<u64>().unwrap_or(0);
            source = "proc_net_dev".to_string();
        }
    }

    let total_bytes = rx_bytes.saturating_add(tx_bytes);
    let used_percent = quota_gb.filter(|q| *q > 0.0).map(|q| {
        let quota_bytes = q * 1024.0 * 1024.0 * 1024.0;
        ((total_bytes as f64 / quota_bytes) * 100.0).min(999.9)
    });
    let warning = used_percent.is_some_and(|pct| pct >= 80.0);

    MonthlyTrafficReport {
        interface: iface,
        source,
        rx_bytes,
        tx_bytes,
        total_bytes,
        quota_gb,
        used_percent,
        warning,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ps_output_rows() {
        let sample = "  123     1 root      12.5  3.4  65536 Ss   02:15:30 /usr/bin/nginx -g daemon off;\n  456   123 www-data   0.2  1.1  12000 S    00:05:12 nginx: worker process\n";
        let rows = parse_ps_output(sample);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].pid, 123);
        assert_eq!(rows[0].cpu_percent, 12.5);
        assert_eq!(rows[0].command, "/usr/bin/nginx -g daemon off;");
        assert_eq!(rows[1].user, "www-data");
    }

    #[test]
    fn validates_signals_and_builds_log_commands() {
        assert_eq!(validate_signal("sigterm").unwrap(), "TERM");
        assert_eq!(validate_signal("KILL").unwrap(), "KILL");
        assert!(validate_signal("STOP").is_err());

        let cmd = build_log_command(&LogStreamSource::File {
            path: "/var/log/nginx/error.log".into(),
            lines: Some(50),
        })
        .unwrap();
        assert!(cmd.contains("tail -n 50 -F"));
        assert!(cmd.contains("/var/log/nginx/error.log"));
    }

    #[test]
    fn parses_monthly_traffic_from_vnstat_and_proc() {
        let proc_out = "__PROC__\nens3 107374182400 214748364800\n";
        let report = parse_traffic_output(proc_out, Some(300.0));
        assert_eq!(report.interface, "ens3");
        assert_eq!(report.source, "proc_net_dev");
        assert_eq!(report.total_bytes, 322_122_547_200);
        assert!(report.warning);
        assert!((report.used_percent.unwrap() - 100.0).abs() < 0.1);
    }
}
