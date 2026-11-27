use std::sync::OnceLock;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{Local, NaiveTime, Utc};
use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use reqwest::Url;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::db::Database;
use crate::error::{AppError, AppResult};
use crate::models::{
    NotificationChannel, NotificationChannelInput, NotificationChannelKind, NotificationDelivery,
    NotificationEventKind, NotificationSeverity,
};
#[cfg(not(test))]
use crate::utils;

#[derive(Debug, Clone)]
pub struct NotificationDispatchEvent {
    pub kind: NotificationEventKind,
    pub severity: NotificationSeverity,
    pub title: String,
    pub message: String,
    pub target: Option<String>,
}

fn shared_client() -> AppResult<&'static Client> {
    static CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();
    let result = CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(8))
            .redirect(Policy::none())
            .user_agent(concat!("ControlVPS/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| format!("无法初始化通知客户端：{error}"))
    });
    match result {
        Ok(client) => Ok(client),
        Err(message) => Err(AppError::Message(message.clone())),
    }
}

pub fn validate_input(
    input: &NotificationChannelInput,
    has_existing_secret: bool,
) -> AppResult<()> {
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(AppError::Message("渠道名称不能为空且不能超过 80 个字符".into()));
    }
    let secret = input
        .secret
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let target = input.target.trim();
    match input.kind {
        NotificationChannelKind::Telegram => {
            if target.is_empty() || target.len() > 128 {
                return Err(AppError::Message("Telegram 需要填写 Chat ID".into()));
            }
            if secret.is_none() && !has_existing_secret {
                return Err(AppError::Message("Telegram 需要填写 Bot Token".into()));
            }
        }
        NotificationChannelKind::Feishu
        | NotificationChannelKind::Dingtalk
        | NotificationChannelKind::Wecom
        | NotificationChannelKind::Webhook => {
            if let Some(url) = secret {
                validate_http_url(url, "Webhook URL")?;
            } else if !has_existing_secret {
                return Err(AppError::Message("需要填写完整的 Webhook URL".into()));
            }
        }
        NotificationChannelKind::Ntfy => {
            validate_http_url(target, "ntfy 主题地址（例如 https://ntfy.sh/my-topic）")?;
        }
        NotificationChannelKind::Bark => {
            let base = if target.is_empty() {
                "https://api.day.app"
            } else {
                target
            };
            validate_http_url(base, "Bark 服务地址")?;
            if secret.is_none() && !has_existing_secret {
                return Err(AppError::Message("Bark 需要填写 Device Key".into()));
            }
        }
    }
    validate_quiet_window(input.quiet_start.as_deref(), input.quiet_end.as_deref())?;
    Ok(())
}

fn validate_http_url(raw: &str, label: &str) -> AppResult<Url> {
    let url = Url::parse(raw)
        .map_err(|_| AppError::Message(format!("{label} 必须是合法的 http:// 或 https:// 地址")))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(AppError::Message(format!(
            "{label} 仅支持 http:// 和 https://，且必须包含主机名"
        )));
    }
    Ok(url)
}

fn validate_quiet_window(start: Option<&str>, end: Option<&str>) -> AppResult<()> {
    let s = start.map(str::trim).filter(|v| !v.is_empty());
    let e = end.map(str::trim).filter(|v| !v.is_empty());
    match (s, e) {
        (None, None) => Ok(()),
        (Some(start_str), Some(end_str)) => {
            parse_hhmm(start_str)
                .ok_or_else(|| AppError::Message("静默开始时间格式应为 HH:MM（如 23:00）".into()))?;
            parse_hhmm(end_str)
                .ok_or_else(|| AppError::Message("静默结束时间格式应为 HH:MM（如 08:00）".into()))?;
            Ok(())
        }
        _ => Err(AppError::Message("静默时段需要同时填写开始和结束时间".into())),
    }
}

fn parse_hhmm(value: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(value.trim(), "%H:%M").ok()
}

pub fn create_channel(
    db: &Database,
    input: &NotificationChannelInput,
) -> AppResult<NotificationChannel> {
    validate_input(input, false)?;
    let id = uuid::Uuid::new_v4().to_string();
    save_channel_secrets(&id, input)?;
    match db.create_notification_channel(&id, input) {
        Ok(channel) => Ok(channel),
        Err(error) => {
            delete_channel_secrets(&id);
            Err(error)
        }
    }
}

pub fn update_channel(
    db: &Database,
    id: &str,
    input: &NotificationChannelInput,
) -> AppResult<NotificationChannel> {
    let existing = db.get_notification_channel(id)?;
    validate_input(input, existing.has_secret)?;
    save_channel_secrets(id, input)?;
    db.update_notification_channel(id, input)
}

pub fn delete_channel(db: &Database, id: &str) -> AppResult<()> {
    db.get_notification_channel(id)?;
    db.delete_notification_channel(id)?;
    delete_channel_secrets(id);
    Ok(())
}

pub fn dispatch_async(db: Database, event: NotificationDispatchEvent) {
    let _ = std::thread::Builder::new()
        .name("controlvps-notify".into())
        .spawn(move || {
            dispatch_blocking(&db, &event);
        });
}

pub fn dispatch_blocking(db: &Database, event: &NotificationDispatchEvent) {
    let channels = match db.list_notification_channels() {
        Ok(channels) => channels,
        Err(_) => return,
    };
    let now_local = Local::now().time();
    for channel in channels {
        if !channel_matches(&channel, event, now_local) {
            continue;
        }
        let _ = deliver_with_retry(db, &channel, event, 3);
    }
}

pub fn send_test(db: &Database, channel_id: &str) -> AppResult<NotificationDelivery> {
    let channel = db.get_notification_channel(channel_id)?;
    let event = NotificationDispatchEvent {
        kind: NotificationEventKind::HostAlert,
        severity: NotificationSeverity::Info,
        title: "ControlVPS 通知渠道测试".into(),
        message: format!(
            "渠道「{}」连接正常。主机告警、服务故障、证书到期、备份失败与 VPS 到期提醒将通过此渠道送达。",
            channel.name
        ),
        target: Some(channel.name.clone()),
    };
    deliver_with_retry(db, &channel, &event, 1)
}

fn deliver_with_retry(
    db: &Database,
    channel: &NotificationChannel,
    event: &NotificationDispatchEvent,
    max_attempts: u32,
) -> AppResult<NotificationDelivery> {
    let mut last_error = String::new();
    let mut attempts = 0;
    for attempt in 1..=max_attempts.max(1) {
        attempts = attempt;
        match send_once(channel, event) {
            Ok(()) => {
                return db.record_notification_delivery(
                    &channel.id,
                    &channel.name,
                    event.kind,
                    event.severity,
                    &event.title,
                    &event.message,
                    true,
                    None,
                    attempts,
                );
            }
            Err(error) => {
                last_error = error.to_string();
                if attempt < max_attempts {
                    let backoff_secs = 1_u64 << (attempt - 1);
                    std::thread::sleep(Duration::from_secs(backoff_secs.min(4)));
                }
            }
        }
    }
    let delivery = db.record_notification_delivery(
        &channel.id,
        &channel.name,
        event.kind,
        event.severity,
        &event.title,
        &event.message,
        false,
        Some(&last_error),
        attempts,
    )?;
    if max_attempts == 1 {
        return Err(AppError::Message(last_error));
    }
    Ok(delivery)
}

fn send_once(channel: &NotificationChannel, event: &NotificationDispatchEvent) -> AppResult<()> {
    let client = shared_client()?;
    let secret = load_channel_secret(&channel.id)?;
    let signing_secret = load_channel_signing_secret(&channel.id)?;
    let severity_tag = severity_label(event.severity);
    let event_tag = event_kind_label(event.kind);
    let target_line = event
        .target
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|value| format!("\n对象：{value}"))
        .unwrap_or_default();
    let plain_body = format!(
        "[{severity_tag} · {event_tag}] {}\n{}{}\n时间：{}",
        event.title,
        event.message,
        target_line,
        Local::now().format("%Y-%m-%d %H:%M:%S")
    );

    let response = match channel.kind {
        NotificationChannelKind::Telegram => {
            let token = secret
                .filter(|value| !value.is_empty())
                .ok_or_else(|| AppError::Message("钥匙串中未找到 Telegram Bot Token".into()))?;
            let url = format!("https://api.telegram.org/bot{token}/sendMessage");
            client
                .post(url)
                .json(&json!({
                    "chat_id": channel.target.trim(),
                    "text": plain_body,
                    "disable_web_page_preview": true,
                }))
                .send()
        }
        NotificationChannelKind::Feishu => {
            let url = secret
                .filter(|value| !value.is_empty())
                .ok_or_else(|| AppError::Message("钥匙串中未找到飞书 Webhook URL".into()))?;
            validate_http_url(&url, "飞书 Webhook URL")?;
            let mut payload = json!({
                "msg_type": "text",
                "content": { "text": plain_body }
            });
            if let Some(sign_key) = signing_secret.filter(|value| !value.is_empty()) {
                let timestamp = Utc::now().timestamp().to_string();
                let key_material = format!("{timestamp}\n{sign_key}");
                let sig = STANDARD.encode(hmac_sha256(key_material.as_bytes(), b""));
                payload["timestamp"] = json!(timestamp);
                payload["sign"] = json!(sig);
            }
            client.post(&url).json(&payload).send()
        }
        NotificationChannelKind::Dingtalk => {
            let mut url = secret
                .filter(|value| !value.is_empty())
                .ok_or_else(|| AppError::Message("钥匙串中未找到钉钉 Webhook URL".into()))?;
            validate_http_url(&url, "钉钉 Webhook URL")?;
            if let Some(sign_key) = signing_secret.filter(|value| !value.is_empty()) {
                let timestamp_ms = Utc::now().timestamp_millis();
                let string_to_sign = format!("{timestamp_ms}\n{sign_key}");
                let sig = STANDARD.encode(hmac_sha256(
                    sign_key.as_bytes(),
                    string_to_sign.as_bytes(),
                ));
                let encoded_sig = percent_encode_query(&sig);
                let sep = if url.contains('?') { '&' } else { '?' };
                url.push_str(&format!("{sep}timestamp={timestamp_ms}&sign={encoded_sig}"));
            }
            client
                .post(&url)
                .json(&json!({
                    "msgtype": "markdown",
                    "markdown": {
                        "title": format!("[{severity_tag}] {}", event.title),
                        "text": format!("### [{severity_tag} · {event_tag}] {}\n\n{}{}\n\n> ControlVPS · {}", event.title, event.message, target_line, Local::now().format("%Y-%m-%d %H:%M:%S")),
                    }
                }))
                .send()
        }
        NotificationChannelKind::Wecom => {
            let url = secret
                .filter(|value| !value.is_empty())
                .ok_or_else(|| AppError::Message("钥匙串中未找到企业微信 Webhook URL".into()))?;
            validate_http_url(&url, "企业微信 Webhook URL")?;
            client
                .post(&url)
                .json(&json!({
                    "msgtype": "markdown",
                    "markdown": {
                        "content": format!("**[{} · {}] {}**\n{}{}\n> {}", severity_tag, event_tag, event.title, event.message, target_line, Local::now().format("%Y-%m-%d %H:%M:%S")),
                    }
                }))
                .send()
        }
        NotificationChannelKind::Ntfy => {
            validate_http_url(channel.target.trim(), "ntfy 地址")?;
            let priority = match event.severity {
                NotificationSeverity::Critical => "urgent",
                NotificationSeverity::Warning => "high",
                NotificationSeverity::Info => "default",
            };
            let mut request = client
                .post(channel.target.trim())
                .header("Title", format!("ControlVPS: {}", event.title))
                .header("Priority", priority)
                .body(plain_body);
            if let Some(token) = secret.filter(|value| !value.is_empty()) {
                request = request.header("Authorization", format!("Bearer {token}"));
            }
            request.send()
        }
        NotificationChannelKind::Bark => {
            let device_key = secret
                .filter(|value| !value.is_empty())
                .ok_or_else(|| AppError::Message("钥匙串中未找到 Bark Device Key".into()))?;
            let base = if channel.target.trim().is_empty() {
                "https://api.day.app"
            } else {
                channel.target.trim().trim_end_matches('/')
            };
            validate_http_url(base, "Bark 服务地址")?;
            let url = format!("{base}/{}", percent_encode_query(&device_key));
            client
                .post(url)
                .json(&json!({
                    "title": format!("[{severity_tag}] {}", event.title),
                    "body": format!("{}{}", event.message, target_line),
                    "group": "ControlVPS",
                }))
                .send()
        }
        NotificationChannelKind::Webhook => {
            let url = secret
                .filter(|value| !value.is_empty())
                .ok_or_else(|| AppError::Message("钥匙串中未找到 Webhook URL".into()))?;
            validate_http_url(&url, "Webhook URL")?;
            let body = serde_json::to_string(&json!({
                "source": "controlvps",
                "eventKind": event.kind,
                "severity": event.severity,
                "title": event.title,
                "message": event.message,
                "target": event.target,
                "timestamp": Utc::now().to_rfc3339(),
            }))
            .map_err(|error| AppError::Message(error.to_string()))?;
            let mut request = client
                .post(&url)
                .header("Content-Type", "application/json")
                .body(body.clone());
            if let Some(sign_key) = signing_secret.filter(|value| !value.is_empty()) {
                let mac = hmac_sha256(sign_key.as_bytes(), body.as_bytes());
                request = request.header("X-ControlVPS-Signature-256", format!("sha256={}", to_hex(&mac)));
            }
            request.send()
        }
    };

    let response = response.map_err(|error| AppError::Message(format!("发送失败：{error}")))?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response
            .text()
            .unwrap_or_default()
            .chars()
            .take(180)
            .collect::<String>();
        return Err(AppError::Message(format!(
            "通知服务返回 HTTP {status}{}",
            if detail.is_empty() {
                String::new()
            } else {
                format!("：{detail}")
            }
        )));
    }
    Ok(())
}

pub fn channel_matches(
    channel: &NotificationChannel,
    event: &NotificationDispatchEvent,
    now_local: NaiveTime,
) -> bool {
    if !channel.enabled {
        return false;
    }
    if !channel.events.is_empty() && !channel.events.contains(&event.kind) {
        return false;
    }
    if severity_rank(event.severity) < severity_rank(channel.min_severity) {
        return false;
    }
    // Quiet hours mute non-critical notifications; critical outages always get through.
    if event.severity != NotificationSeverity::Critical
        && in_quiet_hours(
            now_local,
            channel.quiet_start.as_deref(),
            channel.quiet_end.as_deref(),
        )
    {
        return false;
    }
    true
}

fn severity_rank(severity: NotificationSeverity) -> u8 {
    match severity {
        NotificationSeverity::Info => 0,
        NotificationSeverity::Warning => 1,
        NotificationSeverity::Critical => 2,
    }
}

fn severity_label(severity: NotificationSeverity) -> &'static str {
    match severity {
        NotificationSeverity::Info => "信息",
        NotificationSeverity::Warning => "警告",
        NotificationSeverity::Critical => "严重",
    }
}

fn event_kind_label(kind: NotificationEventKind) -> &'static str {
    match kind {
        NotificationEventKind::HostAlert => "主机监控",
        NotificationEventKind::ServiceAlert => "服务探测",
        NotificationEventKind::CertificateExpiry => "证书预警",
        NotificationEventKind::BackupFailure => "备份异常",
        NotificationEventKind::RunbookFailure => "Runbook 失败",
        NotificationEventKind::ServerExpiry => "VPS 到期",
    }
}

pub fn in_quiet_hours(now: NaiveTime, start: Option<&str>, end: Option<&str>) -> bool {
    let (Some(start), Some(end)) = (start.and_then(parse_hhmm), end.and_then(parse_hhmm)) else {
        return false;
    };
    if start == end {
        return false;
    }
    if start < end {
        now >= start && now < end
    } else {
        // Overnight span, e.g. 23:00 -> 08:00
        now >= start || now < end
    }
}

/// RFC 2104 HMAC-SHA256 implemented directly over `sha2::Sha256`.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;
    let mut key_block = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let hashed = Sha256::digest(key);
        key_block[..32].copy_from_slice(&hashed);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut o_key_pad = [0x5cu8; BLOCK_SIZE];
    let mut i_key_pad = [0x36u8; BLOCK_SIZE];
    for idx in 0..BLOCK_SIZE {
        o_key_pad[idx] ^= key_block[idx];
        i_key_pad[idx] ^= key_block[idx];
    }
    let mut inner = Sha256::new();
    inner.update(i_key_pad);
    inner.update(message);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(o_key_pad);
    outer.update(inner_hash);
    let result = outer.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn percent_encode_query(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(not(test))]
fn save_channel_secrets(id: &str, input: &NotificationChannelInput) -> AppResult<()> {
    if let Some(secret) = input
        .secret
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        utils::set_secret(&format!("notify:{id}:secret"), secret)?;
    }
    if input.clear_signing_secret {
        utils::delete_secret(&format!("notify:{id}:signing"))?;
    } else if let Some(signing) = input
        .signing_secret
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        utils::set_secret(&format!("notify:{id}:signing"), signing)?;
    }
    Ok(())
}

#[cfg(test)]
fn save_channel_secrets(_id: &str, _input: &NotificationChannelInput) -> AppResult<()> {
    Ok(())
}

#[cfg(not(test))]
fn delete_channel_secrets(id: &str) {
    let _ = utils::delete_secret(&format!("notify:{id}:secret"));
    let _ = utils::delete_secret(&format!("notify:{id}:signing"));
}

#[cfg(test)]
fn delete_channel_secrets(_id: &str) {}

#[cfg(not(test))]
fn load_channel_secret(id: &str) -> AppResult<Option<String>> {
    utils::get_secret(&format!("notify:{id}:secret"))
}

#[cfg(test)]
fn load_channel_secret(_id: &str) -> AppResult<Option<String>> {
    Ok(None)
}

#[cfg(not(test))]
fn load_channel_signing_secret(id: &str) -> AppResult<Option<String>> {
    utils::get_secret(&format!("notify:{id}:signing"))
}

#[cfg(test)]
fn load_channel_signing_secret(_id: &str) -> AppResult<Option<String>> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_sha256_matches_rfc4231_test_case_2() {
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            to_hex(&mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn quiet_hours_handles_overnight_and_daytime_ranges() {
        let t2330 = NaiveTime::from_hms_opt(23, 30, 0).unwrap();
        let t0300 = NaiveTime::from_hms_opt(3, 0, 0).unwrap();
        let t1200 = NaiveTime::from_hms_opt(12, 0, 0).unwrap();
        assert!(in_quiet_hours(t2330, Some("23:00"), Some("08:00")));
        assert!(in_quiet_hours(t0300, Some("23:00"), Some("08:00")));
        assert!(!in_quiet_hours(t1200, Some("23:00"), Some("08:00")));

        assert!(in_quiet_hours(t1200, Some("11:00"), Some("14:00")));
        assert!(!in_quiet_hours(t0300, Some("11:00"), Some("14:00")));
    }

    #[test]
    fn routing_filters_events_severity_and_quiet_hours() {
        let channel = NotificationChannel {
            id: "c1".into(),
            name: "ops".into(),
            kind: NotificationChannelKind::Telegram,
            enabled: true,
            target: "12345".into(),
            events: vec![
                NotificationEventKind::HostAlert,
                NotificationEventKind::BackupFailure,
            ],
            min_severity: NotificationSeverity::Warning,
            quiet_start: Some("23:00".into()),
            quiet_end: Some("08:00".into()),
            has_secret: true,
            has_signing_secret: false,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let daytime = NaiveTime::from_hms_opt(14, 0, 0).unwrap();
        let midnight = NaiveTime::from_hms_opt(1, 0, 0).unwrap();

        let warn_host = NotificationDispatchEvent {
            kind: NotificationEventKind::HostAlert,
            severity: NotificationSeverity::Warning,
            title: "CPU".into(),
            message: "high".into(),
            target: None,
        };
        let crit_host = NotificationDispatchEvent {
            severity: NotificationSeverity::Critical,
            ..warn_host.clone()
        };
        let info_host = NotificationDispatchEvent {
            severity: NotificationSeverity::Info,
            ..warn_host.clone()
        };
        let unsubscribed = NotificationDispatchEvent {
            kind: NotificationEventKind::ServerExpiry,
            ..crit_host.clone()
        };

        assert!(channel_matches(&channel, &warn_host, daytime));
        assert!(!channel_matches(&channel, &warn_host, midnight));
        // Critical bypasses quiet hours.
        assert!(channel_matches(&channel, &crit_host, midnight));
        // Info below min_severity=Warning is skipped.
        assert!(!channel_matches(&channel, &info_host, daytime));
        // Unsubscribed event kind is skipped.
        assert!(!channel_matches(&channel, &unsubscribed, daytime));
    }
}
