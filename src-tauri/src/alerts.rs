use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};

use crate::db::Database;
use crate::error::{AppError, AppResult};
use crate::models::{AlertKind, AlertStatus, Server, ServerMetrics};

#[derive(Default)]
struct AlertState {
    active: bool,
    last_triggered: Option<Instant>,
}

#[derive(Default)]
struct RegistryData {
    states: HashMap<(String, AlertKind), AlertState>,
    failures: HashMap<String, u32>,
    hydrated: bool,
}

#[derive(Default)]
pub struct AlertRegistry {
    data: Mutex<RegistryData>,
}

impl AlertRegistry {
    fn data(&self) -> AppResult<std::sync::MutexGuard<'_, RegistryData>> {
        self.data
            .lock()
            .map_err(|_| AppError::Message("告警注册表不可用".into()))
    }

    fn hydrate(&self, db: &Database) -> AppResult<()> {
        let mut data = self.data()?;
        if data.hydrated {
            return Ok(());
        }
        for event in db.list_active_alerts()? {
            data.states.insert(
                (event.server_id, event.kind),
                AlertState {
                    active: true,
                    last_triggered: Some(Instant::now()),
                },
            );
        }
        data.hydrated = true;
        Ok(())
    }
}

pub fn record_success(
    app: &AppHandle,
    db: &Database,
    registry: &AlertRegistry,
    server: &Server,
    metrics: &ServerMetrics,
) -> AppResult<()> {
    registry.hydrate(db)?;
    registry.data()?.failures.remove(&server.id);
    change(
        app,
        db,
        registry,
        server,
        AlertKind::Offline,
        false,
        None,
        None,
        "监控采集已恢复".into(),
        0,
    )?;
    let settings = db.monitoring_settings()?;
    if !settings.alerts_enabled {
        return Ok(());
    }
    let memory = usage(metrics.memory_used, metrics.memory_total);
    let disk = usage(metrics.disk_used, metrics.disk_total);
    for (kind, value, threshold, label) in [
        (
            AlertKind::Cpu,
            metrics.cpu_usage,
            settings.cpu_threshold,
            "CPU",
        ),
        (AlertKind::Memory, memory, settings.memory_threshold, "内存"),
        (AlertKind::Disk, disk, settings.disk_threshold, "系统盘"),
    ] {
        let triggered = value >= threshold;
        let message = if triggered {
            format!("{label} 使用率 {value:.1}%，已达到阈值 {threshold:.1}%")
        } else {
            format!("{label} 使用率回落至 {value:.1}%，低于阈值 {threshold:.1}%")
        };
        change(
            app,
            db,
            registry,
            server,
            kind,
            triggered,
            Some(value),
            Some(threshold),
            message,
            settings.cooldown_minutes,
        )?;
    }
    Ok(())
}

pub fn record_failure(
    app: &AppHandle,
    db: &Database,
    registry: &AlertRegistry,
    server: &Server,
    error: &str,
) -> AppResult<()> {
    registry.hydrate(db)?;
    let settings = db.monitoring_settings()?;
    let failures = {
        let mut data = registry.data()?;
        let count = data.failures.entry(server.id.clone()).or_insert(0);
        *count = count.saturating_add(1);
        *count
    };
    if !settings.alerts_enabled || failures < settings.offline_after {
        return Ok(());
    }
    let detail = error.chars().take(180).collect::<String>();
    change(
        app,
        db,
        registry,
        server,
        AlertKind::Offline,
        true,
        Some(f64::from(failures)),
        Some(f64::from(settings.offline_after)),
        format!("连续 {failures} 次采集失败，服务器可能离线或不可达。最近错误：{detail}"),
        settings.cooldown_minutes,
    )
}

fn change(
    app: &AppHandle,
    db: &Database,
    registry: &AlertRegistry,
    server: &Server,
    kind: AlertKind,
    triggered: bool,
    value: Option<f64>,
    threshold: Option<f64>,
    message: String,
    cooldown_minutes: u32,
) -> AppResult<()> {
    enum Action {
        None,
        Trigger,
        Recover,
    }

    let action = {
        let mut data = registry.data()?;
        let state = data.states.entry((server.id.clone(), kind)).or_default();
        if triggered {
            let cooled_down = state.last_triggered.is_none_or(|time| {
                time.elapsed() >= Duration::from_secs(u64::from(cooldown_minutes) * 60)
            });
            if state.active && !cooled_down {
                Action::None
            } else {
                state.active = true;
                state.last_triggered = Some(Instant::now());
                Action::Trigger
            }
        } else if state.active {
            state.active = false;
            state.last_triggered = None;
            Action::Recover
        } else {
            Action::None
        }
    };
    let status = match action {
        Action::None => return Ok(()),
        Action::Trigger => AlertStatus::Triggered,
        Action::Recover => AlertStatus::Recovered,
    };
    let event = db.record_alert(server, kind, status, message, value, threshold)?;
    let _ = app.emit("monitoring-alert", event);
    Ok(())
}

fn usage(used: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        (used as f64 / total as f64 * 100.0).clamp(0.0, 100.0)
    }
}
