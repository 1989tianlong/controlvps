use std::collections::HashMap;
use std::time::Instant;

use chrono::Utc;

use crate::error::{AppError, AppResult};
use crate::models::ServerMetrics;
use crate::ssh::{execute, SshConnection};

const MONITOR_SCRIPT: &str = r#"
printf 'hostname='; hostname
if [ -r /etc/os-release ]; then . /etc/os-release; printf 'os=%s\n' "$PRETTY_NAME"; else printf 'os='; uname -sr; fi
printf 'uptime='; awk '{print $1}' /proc/uptime
printf 'memory='; awk '/MemTotal/{total=$2} /MemAvailable/{available=$2} END {print total ":" available}' /proc/meminfo
printf 'disk='; df -kP / | awk 'NR==2 {print $2 ":" $3}'
printf 'load='; awk '{print $1 ":" $2 ":" $3}' /proc/loadavg
printf 'cpu_before='; awk 'NR==1 {print $2 ":" $3 ":" $4 ":" $5 ":" $6 ":" $7 ":" $8 ":" $9}' /proc/stat
printf 'network_before='; awk '{gsub(":", " "); if ($1 != "lo" && $2 ~ /^[0-9]+$/) {rx+=$2; tx+=$10}} END {printf "%.0f:%.0f\n", rx, tx}' /proc/net/dev
sleep 1
printf 'cpu_after='; awk 'NR==1 {print $2 ":" $3 ":" $4 ":" $5 ":" $6 ":" $7 ":" $8 ":" $9}' /proc/stat
printf 'network_after='; awk '{gsub(":", " "); if ($1 != "lo" && $2 ~ /^[0-9]+$/) {rx+=$2; tx+=$10}} END {printf "%.0f:%.0f\n", rx, tx}' /proc/net/dev
"#;

pub fn collect(server_id: &str, connection: &SshConnection) -> AppResult<ServerMetrics> {
    let started = Instant::now();
    let output = execute(&connection.session, MONITOR_SCRIPT)?;
    if output.exit_code != 0 {
        return Err(AppError::Message(format!(
            "指标采集失败：{}",
            output.stderr.trim()
        )));
    }
    let values = output
        .stdout
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_string(), value.trim().to_string()))
        .collect::<HashMap<_, _>>();
    let value = |key: &str| -> AppResult<&String> {
        values
            .get(key)
            .ok_or_else(|| AppError::Message(format!("服务器未返回指标：{key}")))
    };
    let (cpu_before, cpu_after) = (cpu(value("cpu_before")?), cpu(value("cpu_after")?));
    let elapsed = started.elapsed().as_secs_f64().max(0.001);
    let (network_before, network_after) = (
        pair(value("network_before")?),
        pair(value("network_after")?),
    );
    let (memory_total_kb, memory_available_kb) = pair(value("memory")?);
    let (disk_total_kb, disk_used_kb) = pair(value("disk")?);
    let load = numbers(value("load")?);
    let memory_total = memory_total_kb.saturating_mul(1024);
    let memory_available = memory_available_kb.saturating_mul(1024);

    Ok(ServerMetrics {
        server_id: server_id.to_string(),
        hostname: value("hostname")?.clone(),
        os: value("os")?.clone(),
        uptime_seconds: number(value("uptime")?),
        cpu_usage: cpu_percentage(cpu_before, cpu_after),
        memory_total,
        memory_used: memory_total.saturating_sub(memory_available),
        disk_total: disk_total_kb.saturating_mul(1024),
        disk_used: disk_used_kb.saturating_mul(1024),
        network_rx_bytes_per_second: network_after.0.saturating_sub(network_before.0) as f64
            / elapsed,
        network_tx_bytes_per_second: network_after.1.saturating_sub(network_before.1) as f64
            / elapsed,
        load_average: [load[0], load[1], load[2]],
        collected_at: Utc::now().to_rfc3339(),
    })
}

fn cpu(value: &str) -> Vec<f64> {
    numbers(value)
}

fn cpu_percentage(before: Vec<f64>, after: Vec<f64>) -> f64 {
    if before.len() < 5 || after.len() < 5 {
        return 0.0;
    }
    let total_before: f64 = before.iter().sum();
    let total_after: f64 = after.iter().sum();
    let idle_before = before[3] + before[4];
    let idle_after = after[3] + after[4];
    let total = (total_after - total_before).max(0.0);
    let idle = (idle_after - idle_before).max(0.0);
    if total == 0.0 {
        0.0
    } else {
        ((total - idle) / total * 100.0).clamp(0.0, 100.0)
    }
}

fn pair(value: &str) -> (u64, u64) {
    let values = value
        .split(':')
        .map(|part| part.trim().parse::<u64>().unwrap_or(0))
        .collect::<Vec<_>>();
    (
        values.first().copied().unwrap_or(0),
        values.get(1).copied().unwrap_or(0),
    )
}

fn numbers(value: &str) -> Vec<f64> {
    value
        .split(':')
        .map(|part| part.trim().parse::<f64>().unwrap_or(0.0))
        .collect()
}

fn number(value: &str) -> f64 {
    value.trim().parse::<f64>().unwrap_or(0.0)
}
