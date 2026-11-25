use std::collections::HashSet;
use std::io::Read;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use keyring::Entry;
use native_tls::TlsConnector;
use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use reqwest::Url;
use tauri::{AppHandle, Emitter};
use x509_parser::parse_x509_certificate;

use crate::db::Database;
use crate::error::{AppError, AppResult};
use crate::models::{
    ServiceCheckResult, ServiceEvent, ServiceMonitor, ServiceMonitorInput, ServiceMonitorKind,
    ServiceNotificationInput, ServiceNotificationSettings, ServiceProbe,
};

const WEBHOOK_ACCOUNT: &str = "service-notifications:webhook-url";
const MAX_HTTP_BODY_BYTES: u64 = 256 * 1024;

#[derive(Clone, Default)]
pub struct ServiceMonitorRegistry {
    running: Arc<Mutex<HashSet<String>>>,
}

impl ServiceMonitorRegistry {
    fn begin(&self, id: &str) -> bool {
        self.running
            .lock()
            .map(|mut running| running.insert(id.to_string()))
            .unwrap_or(false)
    }

    fn finish(&self, id: &str) {
        if let Ok(mut running) = self.running.lock() {
            running.remove(id);
        }
    }
}

struct RunGuard<'a> {
    registry: &'a ServiceMonitorRegistry,
    id: String,
}

impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        self.registry.finish(&self.id);
    }
}

pub fn check(
    app: &AppHandle,
    db: &Database,
    registry: &ServiceMonitorRegistry,
    id: &str,
) -> AppResult<ServiceCheckResult> {
    if !registry.begin(id) {
        return Err(AppError::Message("这个服务正在检查，请稍候".into()));
    }
    let _guard = RunGuard {
        registry,
        id: id.to_string(),
    };
    let monitor = db.get_service_monitor(id)?;
    let probe = probe(&monitor);
    let (view, events) = db.record_service_probe(&monitor, &probe)?;
    let mut notification_error = None;
    for event in &events {
        let _ = app.emit("service-monitor-event", event);
        if notification_error.is_none() {
            if let Err(error) = send_event(db, event) {
                notification_error = Some(format!("服务状态已保存，但 Webhook 发送失败：{error}"));
            }
        }
    }
    let _ = app.emit("service-monitor-checked", &view);
    Ok(ServiceCheckResult {
        view,
        events,
        notification_error,
    })
}

pub fn probe(monitor: &ServiceMonitor) -> ServiceProbe {
    match monitor.kind {
        ServiceMonitorKind::Http => probe_http(monitor),
        ServiceMonitorKind::Tcp => probe_tcp(monitor),
        ServiceMonitorKind::Dns => probe_dns(monitor),
    }
}

fn probe_http(monitor: &ServiceMonitor) -> ServiceProbe {
    let mut probe = empty_probe();
    let url = match http_url(&monitor.target) {
        Ok(url) => url,
        Err(error) => {
            probe.detail = error.to_string();
            return probe;
        }
    };
    let mut certificate_error = None;
    if url.scheme() == "https" {
        let host = url.host_str().unwrap_or_default();
        let port = url.port_or_known_default().unwrap_or(443);
        match certificate_info(host, port, monitor.timeout_seconds) {
            Ok((expires_at, days)) => {
                probe.certificate_expires_at = Some(expires_at);
                probe.certificate_days_remaining = Some(days);
            }
            Err(error) => certificate_error = Some(error.to_string()),
        }
    }
    let client = match Client::builder()
        .timeout(Duration::from_secs(u64::from(monitor.timeout_seconds)))
        .connect_timeout(Duration::from_secs(u64::from(monitor.timeout_seconds)))
        .redirect(Policy::limited(5))
        .user_agent(concat!("ControlVPS/", env!("CARGO_PKG_VERSION")))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            probe.detail = format!("无法初始化 HTTP 客户端：{error}");
            return probe;
        }
    };
    let started = Instant::now();
    let response = match client.get(url.clone()).send() {
        Ok(response) => response,
        Err(error) => {
            probe.latency_ms = elapsed_ms(started);
            probe.detail = format!("HTTP 请求失败：{error}");
            return probe;
        }
    };
    probe.latency_ms = elapsed_ms(started);
    let status = response.status();
    probe.status_code = Some(status.as_u16());
    let final_url = response.url().clone();
    let status_matches = status.as_u16() == monitor.expected_status;
    let mut keyword_matches = true;
    let mut keyword_error = None;
    if let Some(keyword) = monitor.keyword.as_deref().filter(|value| !value.is_empty()) {
        let mut body = Vec::new();
        match response.take(MAX_HTTP_BODY_BYTES).read_to_end(&mut body) {
            Ok(_) => {
                keyword_matches = String::from_utf8_lossy(&body).contains(keyword);
                if !keyword_matches {
                    keyword_error = Some(format!("响应前 256 KiB 中未找到关键字“{keyword}”"));
                }
            }
            Err(error) => {
                keyword_matches = false;
                keyword_error = Some(format!("读取响应失败：{error}"));
            }
        }
    }
    probe.success = status_matches && keyword_matches;
    let mut details = vec![format!("HTTP {}", status.as_u16())];
    if !status_matches {
        details.push(format!("期望状态码 {}", monitor.expected_status));
    }
    if final_url != url {
        details.push(format!("最终地址 {final_url}"));
    }
    if let Some(error) = keyword_error {
        details.push(error);
    } else if monitor
        .keyword
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        details.push("关键字匹配".into());
    }
    if let Some(error) = certificate_error {
        details.push(format!("证书日期读取失败：{error}"));
    }
    probe.detail = details.join(" · ");
    probe
}

fn probe_tcp(monitor: &ServiceMonitor) -> ServiceProbe {
    let mut probe = empty_probe();
    let started = Instant::now();
    let addresses =
        match (monitor.target.as_str(), monitor.port.unwrap_or_default()).to_socket_addrs() {
            Ok(addresses) => addresses.take(6).collect::<Vec<_>>(),
            Err(error) => {
                probe.detail = format!("地址解析失败：{error}");
                return probe;
            }
        };
    if addresses.is_empty() {
        probe.detail = "没有可连接的地址".into();
        return probe;
    }
    let deadline = Duration::from_secs(u64::from(monitor.timeout_seconds));
    let mut errors = Vec::new();
    for address in addresses {
        let remaining = deadline.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            break;
        }
        match TcpStream::connect_timeout(&address, remaining) {
            Ok(stream) => {
                let _ = stream.shutdown(std::net::Shutdown::Both);
                probe.success = true;
                probe.latency_ms = elapsed_ms(started);
                probe.detail = format!("TCP 连接成功 · {address}");
                return probe;
            }
            Err(error) => errors.push(format!("{address}: {error}")),
        }
    }
    probe.latency_ms = elapsed_ms(started);
    probe.detail = if started.elapsed() >= deadline {
        format!("TCP 连接超时（{} 秒）", monitor.timeout_seconds)
    } else {
        format!("TCP 连接失败：{}", errors.join("；"))
    };
    probe
}

fn probe_dns(monitor: &ServiceMonitor) -> ServiceProbe {
    let mut probe = empty_probe();
    let started = Instant::now();
    match (monitor.target.as_str(), 0).to_socket_addrs() {
        Ok(addresses) => {
            let mut addresses = addresses
                .map(|address| address.ip().to_string())
                .collect::<Vec<_>>();
            addresses.sort();
            addresses.dedup();
            probe.latency_ms = elapsed_ms(started);
            probe.success = !addresses.is_empty();
            probe.detail = if addresses.is_empty() {
                "DNS 没有返回地址".into()
            } else {
                format!("解析到 {}", addresses.join("、"))
            };
        }
        Err(error) => {
            probe.latency_ms = elapsed_ms(started);
            probe.detail = format!("DNS 解析失败：{error}");
        }
    }
    probe
}

fn certificate_info(host: &str, port: u16, timeout_seconds: u32) -> AppResult<(String, i32)> {
    let timeout = Duration::from_secs(u64::from(timeout_seconds));
    let address = (host, port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| AppError::Message("证书主机没有可连接地址".into()))?;
    let stream = TcpStream::connect_timeout(&address, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let connector = TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()
        .map_err(|error| AppError::Message(format!("无法初始化 TLS：{error}")))?;
    let stream = connector
        .connect(host, stream)
        .map_err(|error| AppError::Message(format!("TLS 握手失败：{error}")))?;
    let certificate = stream
        .peer_certificate()
        .map_err(|error| AppError::Message(format!("无法读取对端证书：{error}")))?
        .ok_or_else(|| AppError::Message("服务器没有提供证书".into()))?;
    let der = certificate
        .to_der()
        .map_err(|error| AppError::Message(format!("无法导出证书：{error}")))?;
    let (_, parsed) = parse_x509_certificate(&der)
        .map_err(|error| AppError::Message(format!("无法解析证书：{error}")))?;
    let expires = DateTime::from_timestamp(parsed.validity().not_after.timestamp(), 0)
        .ok_or_else(|| AppError::Message("证书到期时间无效".into()))?;
    let seconds = expires.timestamp() - Utc::now().timestamp();
    let days = seconds
        .div_euclid(86_400)
        .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    Ok((expires.to_rfc3339(), days))
}

fn empty_probe() -> ServiceProbe {
    ServiceProbe {
        success: false,
        latency_ms: 0,
        status_code: None,
        detail: String::new(),
        certificate_expires_at: None,
        certificate_days_remaining: None,
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

pub fn validate_input(input: &ServiceMonitorInput) -> AppResult<()> {
    if input.name.trim().is_empty() || input.name.chars().count() > 120 {
        return Err(AppError::Message(
            "服务名称不能为空且不能超过 120 个字符".into(),
        ));
    }
    if !(15..=3600).contains(&input.interval_seconds) {
        return Err(AppError::Message(
            "探测间隔必须在 15 秒到 1 小时之间".into(),
        ));
    }
    if !(1..=30).contains(&input.timeout_seconds) {
        return Err(AppError::Message("探测超时必须在 1 到 30 秒之间".into()));
    }
    if !(1..=20).contains(&input.failure_threshold) {
        return Err(AppError::Message("故障判定次数必须在 1 到 20 之间".into()));
    }
    if !(1..=365).contains(&input.certificate_warning_days) {
        return Err(AppError::Message(
            "证书预警天数必须在 1 到 365 天之间".into(),
        ));
    }
    if input.target.trim().is_empty() || input.target.len() > 2048 {
        return Err(AppError::Message(
            "监控目标不能为空且不能超过 2048 字节".into(),
        ));
    }
    if input
        .keyword
        .as_ref()
        .is_some_and(|value| value.len() > 1024)
    {
        return Err(AppError::Message("响应关键字不能超过 1024 字节".into()));
    }
    match input.kind {
        ServiceMonitorKind::Http => {
            http_url(input.target.trim())?;
            if !(100..=599).contains(&input.expected_status) {
                return Err(AppError::Message(
                    "期望 HTTP 状态码必须在 100 到 599 之间".into(),
                ));
            }
        }
        ServiceMonitorKind::Tcp => {
            if input.target.contains("://") || input.port.is_none_or(|port| port == 0) {
                return Err(AppError::Message(
                    "TCP 监控需要主机名或 IP，以及 1 到 65535 的端口".into(),
                ));
            }
        }
        ServiceMonitorKind::Dns => {
            if input.target.contains("://") || input.target.contains('/') {
                return Err(AppError::Message(
                    "DNS 监控目标只需填写主机名，例如 example.com".into(),
                ));
            }
        }
    }
    Ok(())
}

fn http_url(value: &str) -> AppResult<Url> {
    let url = Url::parse(value).map_err(|_| {
        AppError::Message("HTTP 监控需要完整 URL，例如 https://example.com/health".into())
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(AppError::Message(
            "监控 URL 仅支持 http:// 和 https://，且必须包含主机名".into(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::Message(
            "监控 URL 不能包含明文用户名或密码".into(),
        ));
    }
    Ok(url)
}

pub fn notification_settings(db: &Database) -> AppResult<ServiceNotificationSettings> {
    Ok(ServiceNotificationSettings {
        enabled: db.service_webhook_enabled()?,
        has_webhook_url: webhook_url()?.is_some(),
    })
}

pub fn save_notification_settings(
    db: &Database,
    input: &ServiceNotificationInput,
) -> AppResult<ServiceNotificationSettings> {
    if input.clear_url {
        delete_webhook_url()?;
    } else if let Some(url) = input
        .url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        validate_webhook_url(url)?;
        webhook_entry()?
            .set_password(url)
            .map_err(|error| AppError::Keyring(error.to_string()))?;
    }
    let has_url = webhook_url()?.is_some();
    if input.enabled && !has_url {
        return Err(AppError::Message(
            "启用 Webhook 前需要填写有效的 HTTPS 或 HTTP 地址".into(),
        ));
    }
    db.update_service_webhook_enabled(input.enabled)?;
    notification_settings(db)
}

pub fn test_webhook(_db: &Database) -> AppResult<()> {
    let url = webhook_url()?.ok_or_else(|| AppError::Message("尚未保存 Webhook URL".into()))?;
    post_webhook(
        &url,
        serde_json::json!({
            "source": "controlvps",
            "version": env!("CARGO_PKG_VERSION"),
            "type": "test",
            "status": "triggered",
            "message": "ControlVPS Webhook 测试成功",
            "checkedAt": Utc::now().to_rfc3339(),
        }),
    )
}

pub fn webhook_url() -> AppResult<Option<String>> {
    match webhook_entry()?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(AppError::Keyring(error.to_string())),
    }
}

pub fn restore_webhook(db: &Database, enabled: bool, url: Option<&str>) -> AppResult<()> {
    if let Some(url) = url {
        validate_webhook_url(url)?;
        webhook_entry()?
            .set_password(url)
            .map_err(|error| AppError::Keyring(error.to_string()))?;
    }
    let effective = enabled && webhook_url()?.is_some();
    db.update_service_webhook_enabled(effective)
}

fn delete_webhook_url() -> AppResult<()> {
    match webhook_entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(AppError::Keyring(error.to_string())),
    }
}

fn webhook_entry() -> AppResult<Entry> {
    Entry::new("com.controlvps.app", WEBHOOK_ACCOUNT)
        .map_err(|error| AppError::Keyring(error.to_string()))
}

fn send_event(db: &Database, event: &ServiceEvent) -> AppResult<()> {
    if !db.service_webhook_enabled()? {
        return Ok(());
    }
    let Some(url) = webhook_url()? else {
        return Ok(());
    };
    post_webhook(
        &url,
        serde_json::json!({
            "source": "controlvps",
            "version": env!("CARGO_PKG_VERSION"),
            "type": "service_event",
            "monitor": { "id": event.monitor_id, "name": event.monitor_name },
            "kind": event.kind,
            "status": event.status,
            "message": event.message,
            "checkedAt": event.created_at,
        }),
    )
}

fn post_webhook(url: &str, payload: serde_json::Value) -> AppResult<()> {
    validate_webhook_url(url)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(10))
        .redirect(Policy::none())
        .user_agent(concat!("ControlVPS/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| AppError::Message(format!("无法初始化 Webhook 客户端：{error}")))?;
    let response = client
        .post(url)
        .header("Content-Type", "application/json")
        .body(payload.to_string())
        .send()
        .map_err(|error| AppError::Message(format!("Webhook 请求失败：{error}")))?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(AppError::Message(format!(
            "Webhook 返回 HTTP {}，请检查接收端",
            response.status().as_u16()
        )))
    }
}

fn validate_webhook_url(value: &str) -> AppResult<Url> {
    let url = http_url(value)?;
    if url.as_str().len() > 2048 {
        return Err(AppError::Message("Webhook URL 不能超过 2048 字节".into()));
    }
    Ok(url)
}
