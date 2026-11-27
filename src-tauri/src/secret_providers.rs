use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

const CLI_TIMEOUT: Duration = Duration::from_secs(10);
const DETECT_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretProviderStatus {
    pub provider: String,
    pub label: String,
    pub available: bool,
    pub cli_path: Option<String>,
    pub version: Option<String>,
    pub signed_in: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretRefTestResult {
    pub ok: bool,
    pub provider: String,
    pub preview: String,
    pub elapsed_ms: u64,
}

pub fn is_secret_ref(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.starts_with("op://") || trimmed.starts_with("bw://")
}

pub fn detect_providers() -> Vec<SecretProviderStatus> {
    vec![detect_onepassword(), detect_bitwarden()]
}

pub fn resolve_secret_value(raw: &str) -> AppResult<String> {
    if !is_secret_ref(raw) {
        return Ok(raw.to_string());
    }
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("op://") {
        if rest.is_empty() || trimmed.chars().any(|c| c.is_control()) {
            return Err(AppError::Message(
                "无效的 1Password 凭据引用，格式应为 op://Vault/Item/Field".into(),
            ));
        }
        return resolve_onepassword(trimmed);
    }
    if let Some(rest) = trimmed.strip_prefix("bw://") {
        if rest.is_empty() || trimmed.chars().any(|c| c.is_control()) {
            return Err(AppError::Message(
                "无效的 Bitwarden 凭据引用，格式应为 bw://条目名/password".into(),
            ));
        }
        return resolve_bitwarden(rest);
    }
    Ok(raw.to_string())
}

pub fn test_secret_ref(reference: &str) -> AppResult<SecretRefTestResult> {
    let trimmed = reference.trim();
    let provider = if trimmed.starts_with("op://") {
        "onepassword"
    } else if trimmed.starts_with("bw://") {
        "bitwarden"
    } else {
        return Err(AppError::Message(
            "凭据引用必须以 op:// 或 bw:// 开头".into(),
        ));
    };
    let started = Instant::now();
    let mut secret = resolve_secret_value(trimmed)?;
    let char_count = secret.chars().count();
    crate::utils::zeroize_string(&mut secret);
    Ok(SecretRefTestResult {
        ok: true,
        provider: provider.to_string(),
        preview: format!("已解析（{char_count} 个字符，不在界面显示明文）"),
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

fn detect_onepassword() -> SecretProviderStatus {
    let Some(cli) = find_cli("op") else {
        return SecretProviderStatus {
            provider: "onepassword".into(),
            label: "1Password CLI (op)".into(),
            available: false,
            cli_path: None,
            version: None,
            signed_in: false,
            detail: "未检测到 op CLI，可通过 brew install 1password-cli 安装".into(),
        };
    };
    let version = run_cli_with_timeout(&cli, &["--version"], DETECT_TIMEOUT)
        .ok()
        .map(|out| out.trim().to_string())
        .filter(|v| !v.is_empty());
    let (signed_in, detail) = match run_cli_with_timeout(&cli, &["whoami", "--format=json"], DETECT_TIMEOUT) {
        Ok(out) => {
            let email = serde_json::from_str::<serde_json::Value>(&out)
                .ok()
                .and_then(|v| v.get("email").and_then(|e| e.as_str()).map(ToString::to_string));
            (
                true,
                email
                    .map(|e| format!("已登录 ({e})"))
                    .unwrap_or_else(|| "已登录 1Password".into()),
            )
        }
        Err(_) => (
            false,
            "已安装 op CLI，但尚未登录或在 1Password 桌面端开启 CLI 集成".into(),
        ),
    };
    SecretProviderStatus {
        provider: "onepassword".into(),
        label: "1Password CLI (op)".into(),
        available: true,
        cli_path: Some(cli.display().to_string()),
        version,
        signed_in,
        detail,
    }
}

fn detect_bitwarden() -> SecretProviderStatus {
    let Some(cli) = find_cli("bw") else {
        return SecretProviderStatus {
            provider: "bitwarden".into(),
            label: "Bitwarden CLI (bw)".into(),
            available: false,
            cli_path: None,
            version: None,
            signed_in: false,
            detail: "未检测到 bw CLI，可通过 brew install bitwarden-cli 安装".into(),
        };
    };
    let version = run_cli_with_timeout(&cli, &["--version"], DETECT_TIMEOUT)
        .ok()
        .map(|out| out.trim().to_string())
        .filter(|v| !v.is_empty());
    let (signed_in, detail) = match run_cli_with_timeout(&cli, &["status"], DETECT_TIMEOUT) {
        Ok(out) => {
            let status = serde_json::from_str::<serde_json::Value>(&out)
                .ok()
                .and_then(|v| v.get("status").and_then(|s| s.as_str()).map(ToString::to_string))
                .unwrap_or_else(|| "unknown".into());
            match status.as_str() {
                "unlocked" => (true, "Bitwarden 保险库已解锁".into()),
                "locked" => (false, "Bitwarden 已登录但处于锁定状态（请设置 BW_SESSION 或运行 bw unlock）".into()),
                "unauthenticated" => (false, "尚未登录 Bitwarden（请运行 bw login）".into()),
                other => (false, format!("状态：{other}")),
            }
        }
        Err(_) => (false, "已安装 bw CLI，无法读取状态".into()),
    };
    SecretProviderStatus {
        provider: "bitwarden".into(),
        label: "Bitwarden CLI (bw)".into(),
        available: true,
        cli_path: Some(cli.display().to_string()),
        version,
        signed_in,
        detail,
    }
}

fn resolve_onepassword(reference: &str) -> AppResult<String> {
    let cli = find_cli("op").ok_or_else(|| {
        AppError::Message("未找到 1Password CLI (op)，请先安装并开启 CLI 集成".into())
    })?;
    let out = run_cli_with_timeout(&cli, &["read", "--no-newline", reference], CLI_TIMEOUT)
        .map_err(|err| AppError::Message(format!("1Password 取密失败：{err}")))?;
    if out.is_empty() {
        return Err(AppError::Message("1Password 返回了空凭据".into()));
    }
    Ok(out)
}

#[derive(Debug, PartialEq, Eq)]
enum BwField {
    Password,
    Notes,
    Username,
    Custom(String),
}

fn parse_bw_ref(rest: &str) -> AppResult<(String, BwField)> {
    let parts: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return Err(AppError::Message(
            "Bitwarden 引用缺少条目名称，格式：bw://<条目名或ID>[/password|notes|username|field/<字段名>]".into(),
        ));
    }
    let item = parts[0].to_string();
    let field = match parts.get(1).copied() {
        None | Some("password") => BwField::Password,
        Some("notes") => BwField::Notes,
        Some("username") => BwField::Username,
        Some("field") => {
            let name = parts.get(2).copied().unwrap_or("").trim();
            if name.is_empty() {
                return Err(AppError::Message("Bitwarden 自定义字段名不能为空".into()));
            }
            BwField::Custom(name.to_string())
        }
        Some(other) => BwField::Custom(other.to_string()),
    };
    Ok((item, field))
}

fn resolve_bitwarden(rest: &str) -> AppResult<String> {
    let cli = find_cli("bw").ok_or_else(|| {
        AppError::Message("未找到 Bitwarden CLI (bw)，请先安装并解锁保险库".into())
    })?;
    let (item, field) = parse_bw_ref(rest)?;
    match field {
        BwField::Password => {
            let out = run_cli_with_timeout(&cli, &["get", "password", &item], CLI_TIMEOUT)
                .map_err(|err| AppError::Message(format!("Bitwarden 获取密码失败：{err}")))?;
            Ok(out.trim_end_matches(['\r', '\n']).to_string())
        }
        BwField::Notes => {
            let out = run_cli_with_timeout(&cli, &["get", "notes", &item], CLI_TIMEOUT)
                .map_err(|err| AppError::Message(format!("Bitwarden 获取备注失败：{err}")))?;
            Ok(out.trim_end_matches(['\r', '\n']).to_string())
        }
        BwField::Username => {
            let out = run_cli_with_timeout(&cli, &["get", "username", &item], CLI_TIMEOUT)
                .map_err(|err| AppError::Message(format!("Bitwarden 获取用户名失败：{err}")))?;
            Ok(out.trim_end_matches(['\r', '\n']).to_string())
        }
        BwField::Custom(field_name) => {
            let out = run_cli_with_timeout(&cli, &["get", "item", &item], CLI_TIMEOUT)
                .map_err(|err| AppError::Message(format!("Bitwarden 获取条目失败：{err}")))?;
            let value: serde_json::Value = serde_json::from_str(&out)
                .map_err(|err| AppError::Message(format!("解析 Bitwarden 条目失败：{err}")))?;
            if let Some(fields) = value.get("fields").and_then(|v| v.as_array()) {
                for entry in fields {
                    if entry.get("name").and_then(|n| n.as_str()) == Some(field_name.as_str()) {
                        if let Some(val) = entry.get("value").and_then(|v| v.as_str()) {
                            return Ok(val.to_string());
                        }
                    }
                }
            }
            Err(AppError::Message(format!(
                "Bitwarden 条目 {item} 中未找到自定义字段 {field_name}"
            )))
        }
    }
}

fn find_cli(name: &str) -> Option<PathBuf> {
    if let Ok(path_env) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_env) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    for dir in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        let candidate = PathBuf::from(dir).join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn run_cli_with_timeout(cli: &PathBuf, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut child = Command::new(cli)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("启动 {} 失败：{err}", cli.display()))?;

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child
                    .wait_with_output()
                    .map_err(|err| format!("读取命令输出失败：{err}"))?;
                if output.status.success() {
                    return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
                }
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(if stderr.is_empty() {
                    format!("退出码 {}", output.status.code().unwrap_or(-1))
                } else {
                    stderr
                });
            }
            Ok(None) => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("命令超时（{} 秒）", timeout.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(40));
            }
            Err(err) => {
                let _ = child.kill();
                return Err(format!("等待命令结束失败：{err}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_secret_ref_prefixes() {
        assert!(is_secret_ref("op://Personal/VPS-1/password"));
        assert!(is_secret_ref("bw://my-vps/password"));
        assert!(!is_secret_ref("plain-password-123"));
        assert!(!is_secret_ref(""));
    }

    #[test]
    fn parses_bitwarden_refs() {
        assert_eq!(
            parse_bw_ref("prod-vps").unwrap(),
            ("prod-vps".into(), BwField::Password)
        );
        assert_eq!(
            parse_bw_ref("prod-vps/notes").unwrap(),
            ("prod-vps".into(), BwField::Notes)
        );
        assert_eq!(
            parse_bw_ref("prod-vps/field/ssh_passphrase").unwrap(),
            ("prod-vps".into(), BwField::Custom("ssh_passphrase".into()))
        );
        assert!(parse_bw_ref("").is_err());
    }
}
