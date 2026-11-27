use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use chrono::Utc;
use uuid::Uuid;

use crate::db::Database;
use crate::error::{AppError, AppResult};
use crate::models::{BenchmarkEndpointCheck, ServerBenchmark};
use crate::runbook;
use crate::ssh;

const BENCHMARK_SCRIPT: &str = r#"
set -u
echo "__CPU_MODEL__=$(awk -F: '/model name|Model name|Hardware/ {gsub(/^[ \t]+/,"",$2); print $2; exit}' /proc/cpuinfo 2>/dev/null || uname -m)"
echo "__CPU_CORES__=$(nproc 2>/dev/null || grep -c ^processor /proc/cpuinfo 2>/dev/null || echo 1)"
echo "__ARCH__=$(uname -m 2>/dev/null || echo unknown)"
echo "__KERNEL__=$(uname -r 2>/dev/null || echo unknown)"
echo "__OS__=$(. /etc/os-release 2>/dev/null && printf '%s' "${PRETTY_NAME:-Linux}" || uname -s)"
echo "__VIRT__=$(systemd-detect-virt 2>/dev/null || echo unknown)"
echo "__MEM_MB__=$(awk '/MemTotal:/ {printf "%d", $2/1024}' /proc/meminfo 2>/dev/null || echo 0)"
echo "__DISK_GB__=$(df -BG / 2>/dev/null | awk 'NR==2 {gsub(/G/,"",$2); print $2}' || echo 0)"
echo "__AES_NI__=$(grep -qiE 'aes|pmull' /proc/cpuinfo 2>/dev/null && echo 1 || echo 0)"

# Lightweight CPU hash throughput (64 MiB SHA-256)
cpu_ms=$( (
  t0=$(date +%s%3N 2>/dev/null || echo 0)
  dd if=/dev/zero bs=1M count=64 status=none 2>/dev/null | sha256sum >/dev/null 2>&1 || true
  t1=$(date +%s%3N 2>/dev/null || echo 0)
  if [ "$t0" -gt 0 ] && [ "$t1" -gt "$t0" ]; then echo $((t1 - t0)); else echo 0; fi
) )
echo "__CPU_MS__=$cpu_ms"

# Lightweight sequential disk write (64 MiB with fdatasync, always cleaned up)
tmp_bench="/tmp/.controlvps-bench-$$"
trap 'rm -f "$tmp_bench"' EXIT
disk_ms=$( (
  t0=$(date +%s%3N 2>/dev/null || echo 0)
  dd if=/dev/zero of="$tmp_bench" bs=1M count=64 conv=fdatasync status=none 2>/dev/null || dd if=/dev/zero of="$tmp_bench" bs=1M count=64 status=none 2>/dev/null || true
  t1=$(date +%s%3N 2>/dev/null || echo 0)
  rm -f "$tmp_bench"
  if [ "$t0" -gt 0 ] && [ "$t1" -gt "$t0" ]; then echo $((t1 - t0)); else echo 0; fi
) )
echo "__DISK_MS__=$disk_ms"

# IPv4 / IPv6 & ASN info (best effort, short timeout)
if command -v curl >/dev/null 2>&1; then
  v4=$(curl -4 -s -m 4 https://api.ipify.org 2>/dev/null | tr -d '\r\n' || true)
  v6=$(curl -6 -s -m 3 https://api6.ipify.org 2>/dev/null | tr -d '\r\n' || true)
  ipinfo=$(curl -4 -s -m 4 https://ipinfo.io/json 2>/dev/null | tr '\n' ' ' || true)
  echo "__IPV4__=$v4"
  echo "__IPV6__=$v6"
  echo "__IPINFO__=$ipinfo"
  for pair in \
    "GitHub|https://api.github.com" \
    "DockerHub|https://registry-1.docker.io/v2/" \
    "PyPI|https://pypi.org/simple/" \
    "OpenAI|https://api.openai.com/v1/models" \
    "Anthropic|https://api.anthropic.com" \
    "Cloudflare|https://1.1.1.1/cdn-cgi/trace"
  do
    name="${pair%%|*}"
    url="${pair#*|}"
    out=$(curl -o /dev/null -s -m 5 -w '%{http_code}|%{time_total}' "$url" 2>/dev/null || echo '000|0')
    echo "__PROBE__=${name}|${out}"
  done
else
  echo "__IPV4__="
  echo "__IPV6__="
  echo "__IPINFO__="
fi
"#;

pub fn run(
    db: &Database,
    server_id: &str,
    known_hosts: &Path,
    allow_unknown_host: bool,
) -> AppResult<ServerBenchmark> {
    let job = runbook::connection_job(db, server_id)?;
    let local_rtt_ms = measure_local_tcp_rtt(&job.server.host, job.server.port);
    let connection = ssh::connect(
        &job.server,
        &job.credentials,
        &job.jumps,
        known_hosts,
        allow_unknown_host,
    )?;
    let started = Instant::now();
    let cancel = AtomicBool::new(false);
    let exec = ssh::execute_controlled(
        &connection.session,
        BENCHMARK_SCRIPT,
        Duration::from_secs(60),
        &cancel,
    )?;
    let stdout = match exec {
        ssh::ControlledExecOutput::Completed(out) => out.stdout,
        ssh::ControlledExecOutput::TimedOut { stdout, .. } => {
            if stdout.trim().is_empty() {
                return Err(AppError::Message("验机超时（60 秒），请检查服务器网络或负载".into()));
            }
            stdout
        }
        ssh::ControlledExecOutput::Cancelled { .. } => {
            return Err(AppError::Message("验机已取消".into()));
        }
    };
    let mut benchmark = parse_benchmark_output(
        &job.server.id,
        &job.server.name,
        local_rtt_ms,
        started.elapsed().as_millis() as u64,
        &stdout,
    );
    benchmark.id = Uuid::new_v4().to_string();
    benchmark.created_at = Utc::now().to_rfc3339();
    db.save_server_benchmark(&benchmark)?;
    let _ = db.log_operation(
        &job.server.id,
        "一键验机",
        true,
        &format!(
            "综合评分 {} · CPU {:.0} MB/s · 磁盘 {:.0} MB/s · 延迟 {}",
            benchmark.score,
            benchmark.cpu_mb_per_sec,
            benchmark.disk_write_mb_per_sec,
            benchmark
                .local_rtt_ms
                .map(|v| format!("{v} ms"))
                .unwrap_or_else(|| "未知".into())
        ),
    );
    Ok(benchmark)
}

fn measure_local_tcp_rtt(host: &str, port: u16) -> Option<u64> {
    let addr = (host, port).to_socket_addrs().ok()?.next()?;
    let mut best: Option<u64> = None;
    for _ in 0..3 {
        let started = Instant::now();
        if TcpStream::connect_timeout(&addr, Duration::from_secs(3)).is_ok() {
            let ms = started.elapsed().as_millis() as u64;
            best = Some(best.map_or(ms, |cur| cur.min(ms)));
        }
    }
    best
}

pub fn parse_benchmark_output(
    server_id: &str,
    server_name: &str,
    local_rtt_ms: Option<u64>,
    duration_ms: u64,
    raw: &str,
) -> ServerBenchmark {
    let mut cpu_model = String::from("未知");
    let mut cpu_cores: u32 = 1;
    let mut arch = String::from("unknown");
    let mut kernel = String::from("unknown");
    let mut os = String::from("Linux");
    let mut virtualization = String::from("unknown");
    let mut memory_mb: u64 = 0;
    let mut disk_gb: u64 = 0;
    let mut aes_ni = false;
    let mut cpu_ms: u64 = 0;
    let mut disk_ms: u64 = 0;
    let mut public_ipv4: Option<String> = None;
    let mut has_ipv6 = false;
    let mut asn: Option<String> = None;
    let mut location: Option<String> = None;
    let mut endpoints: Vec<BenchmarkEndpointCheck> = Vec::new();

    for line in raw.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("__CPU_MODEL__=") {
            if !v.trim().is_empty() {
                cpu_model = v.trim().to_string();
            }
        } else if let Some(v) = line.strip_prefix("__CPU_CORES__=") {
            cpu_cores = v.trim().parse().unwrap_or(1).max(1);
        } else if let Some(v) = line.strip_prefix("__ARCH__=") {
            arch = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("__KERNEL__=") {
            kernel = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("__OS__=") {
            if !v.trim().is_empty() {
                os = v.trim().to_string();
            }
        } else if let Some(v) = line.strip_prefix("__VIRT__=") {
            virtualization = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("__MEM_MB__=") {
            memory_mb = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("__DISK_GB__=") {
            disk_gb = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("__AES_NI__=") {
            aes_ni = v.trim() == "1";
        } else if let Some(v) = line.strip_prefix("__CPU_MS__=") {
            cpu_ms = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("__DISK_MS__=") {
            disk_ms = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("__IPV4__=") {
            let trimmed = v.trim();
            if !trimmed.is_empty() && trimmed.len() <= 45 {
                public_ipv4 = Some(trimmed.to_string());
            }
        } else if let Some(v) = line.strip_prefix("__IPV6__=") {
            has_ipv6 = !v.trim().is_empty() && v.contains(':');
        } else if let Some(v) = line.strip_prefix("__IPINFO__=") {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(v) {
                asn = json
                    .get("org")
                    .and_then(|x| x.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                let city = json.get("city").and_then(|x| x.as_str()).unwrap_or("");
                let country = json.get("country").and_then(|x| x.as_str()).unwrap_or("");
                let combined = [city, country]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ");
                if !combined.is_empty() {
                    location = Some(combined);
                }
            }
        } else if let Some(v) = line.strip_prefix("__PROBE__=") {
            let parts: Vec<&str> = v.split('|').collect();
            if parts.len() == 3 {
                let code: u16 = parts[1].trim().parse().unwrap_or(0);
                let secs: f64 = parts[2].trim().parse().unwrap_or(0.0);
                let reachable = (100..=499).contains(&code);
                endpoints.push(BenchmarkEndpointCheck {
                    name: parts[0].trim().to_string(),
                    reachable,
                    status_code: (code > 0).then_some(code),
                    latency_ms: (secs * 1000.0).round() as u64,
                });
            }
        }
    }

    let cpu_mb_per_sec = if cpu_ms > 0 {
        (64.0 * 1000.0 / cpu_ms as f64).min(10_000.0)
    } else {
        0.0
    };
    let disk_write_mb_per_sec = if disk_ms > 0 {
        (64.0 * 1000.0 / disk_ms as f64).min(20_000.0)
    } else {
        0.0
    };

    let score = compute_score(
        cpu_mb_per_sec,
        disk_write_mb_per_sec,
        local_rtt_ms,
        has_ipv6,
        &endpoints,
    );

    ServerBenchmark {
        id: String::new(),
        server_id: server_id.to_string(),
        server_name: server_name.to_string(),
        cpu_model,
        cpu_cores,
        arch,
        kernel,
        os,
        virtualization,
        memory_mb,
        disk_gb,
        aes_ni,
        cpu_mb_per_sec: (cpu_mb_per_sec * 10.0).round() / 10.0,
        disk_write_mb_per_sec: (disk_write_mb_per_sec * 10.0).round() / 10.0,
        local_rtt_ms,
        public_ipv4: public_ipv4.map(|ip| mask_ip(&ip)),
        has_ipv6,
        asn,
        location,
        endpoints,
        score,
        duration_ms,
        created_at: String::new(),
    }
}

fn compute_score(
    cpu_mb_s: f64,
    disk_mb_s: f64,
    local_rtt_ms: Option<u64>,
    has_ipv6: bool,
    endpoints: &[BenchmarkEndpointCheck],
) -> u32 {
    // CPU (30 pts): 400+ MB/s SHA-256 = full marks
    let cpu_pts = ((cpu_mb_s / 400.0).clamp(0.0, 1.0) * 30.0).round() as u32;
    // Disk (30 pts): 350+ MB/s fdatasync write = full marks
    let disk_pts = ((disk_mb_s / 350.0).clamp(0.0, 1.0) * 30.0).round() as u32;
    // Local RTT (20 pts): <=40ms = 20, 40..300ms linear decay
    let rtt_pts = match local_rtt_ms {
        Some(ms) if ms <= 40 => 20,
        Some(ms) if ms >= 300 => 4,
        Some(ms) => (20.0 - ((ms - 40) as f64 / 260.0) * 16.0).round() as u32,
        None => 10,
    };
    // Endpoint reachability (15 pts) + IPv6 bonus (5 pts)
    let ep_pts = if endpoints.is_empty() {
        8
    } else {
        let ok = endpoints.iter().filter(|e| e.reachable).count();
        ((ok as f64 / endpoints.len() as f64) * 15.0).round() as u32
    };
    let ipv6_pts = if has_ipv6 { 5 } else { 0 };
    (cpu_pts + disk_pts + rtt_pts + ep_pts + ipv6_pts).min(100)
}

fn mask_ip(ip: &str) -> String {
    let parts: Vec<&str> = ip.split('.').collect();
    if parts.len() == 4 {
        format!("{}.{}.{}.xxx", parts[0], parts[1], parts[2])
    } else {
        ip.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_benchmark_output_and_masks_ip() {
        let sample = "\
__CPU_MODEL__=AMD EPYC 7763
__CPU_CORES__=2
__ARCH__=x86_64
__KERNEL__=6.8.0
__OS__=Ubuntu 24.04 LTS
__VIRT__=kvm
__MEM_MB__=4096
__DISK_GB__=80
__AES_NI__=1
__CPU_MS__=160
__DISK_MS__=200
__IPV4__=203.0.113.42
__IPV6__=2001:db8::1
__IPINFO__={\"org\":\"AS24940 Hetzner Online GmbH\",\"city\":\"Falkenstein\",\"country\":\"DE\"}
__PROBE__=GitHub|200|0.120
__PROBE__=DockerHub|401|0.095
__PROBE__=OpenAI|401|0.150
";
        let result = parse_benchmark_output("s1", "hetzner-eu", Some(35), 1800, sample);
        assert_eq!(result.cpu_model, "AMD EPYC 7763");
        assert_eq!(result.cpu_cores, 2);
        assert_eq!(result.virtualization, "kvm");
        assert!(result.aes_ni);
        assert_eq!(result.cpu_mb_per_sec, 400.0);
        assert_eq!(result.disk_write_mb_per_sec, 320.0);
        assert_eq!(result.public_ipv4.as_deref(), Some("203.0.113.xxx"));
        assert!(result.has_ipv6);
        assert_eq!(result.asn.as_deref(), Some("AS24940 Hetzner Online GmbH"));
        assert_eq!(result.location.as_deref(), Some("Falkenstein, DE"));
        assert_eq!(result.endpoints.len(), 3);
        assert!(result.endpoints.iter().all(|e| e.reachable));
        assert!(result.score >= 90);
    }
}
