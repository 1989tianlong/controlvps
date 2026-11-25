use std::collections::{HashMap, HashSet};

use chrono::Utc;
use ssh2::Session;

use crate::error::{AppError, AppResult};
use crate::models::{
    FirewallStatus, ListeningPort, PackageUpdate, PackageUpdateKind, SecurityFinding,
    SecurityReport, SecuritySeverity,
};
use crate::ssh::{self, ExecOutput};

const IDENTITY_SCRIPT: &str = r#"
printf 'hostname='; hostname
if [ -r /etc/os-release ]; then . /etc/os-release; printf 'os=%s\n' "$PRETTY_NAME"; else printf 'os='; uname -sr; fi
if command -v apt-get >/dev/null 2>&1 && command -v apt >/dev/null 2>&1; then printf 'manager=apt\n';
elif command -v dnf >/dev/null 2>&1; then printf 'manager=dnf\n';
elif command -v yum >/dev/null 2>&1; then printf 'manager=yum\n';
elif command -v apk >/dev/null 2>&1; then printf 'manager=apk\n';
else printf 'manager=unknown\n'; fi
"#;

const REBOOT_SCRIPT: &str = r#"
if [ -r /var/run/reboot-required ]; then
  printf 'required=yes\nreason='; cat /var/run/reboot-required | tr '\n' ' '; printf '\n'
elif command -v needs-restarting >/dev/null 2>&1; then
  reboot_output=$(needs-restarting -r 2>&1)
  status=$?
  if [ "$status" -eq 1 ]; then printf 'required=yes\nreason=核心库或内核已更新，工具报告需要重启\n';
  elif [ "$status" -eq 0 ]; then printf 'required=no\nreason=无需重启\n';
  else printf 'required=unknown\nreason='; printf '%s' "$reboot_output" | tr '\n' ' '; printf '\n'; fi
else
  printf 'required=no\nreason=当前系统未报告重启需求\n'
fi
"#;

const SSH_CONFIG_FALLBACK: &str = r#"
awk 'BEGIN { FS="[ \t]+" }
  /^[ \t]*(#|$)/ { next }
  {
    key=tolower($1); value=$2
    if (key ~ /^(passwordauthentication|permitrootlogin|pubkeyauthentication|permitemptypasswords|allowtcpforwarding|x11forwarding|maxauthtries)$/ && !seen[key]++) print key, value
  }' /etc/ssh/sshd_config 2>/dev/null
"#;

const FAIL2BAN_SCRIPT: &str = r#"
installed=no
if command -v fail2ban-client >/dev/null 2>&1; then installed=yes; fi
state=unknown
if command -v systemctl >/dev/null 2>&1; then state=$(systemctl is-active fail2ban 2>/dev/null || true); fi
if [ "$state" != active ] && command -v rc-service >/dev/null 2>&1; then
  if rc-service fail2ban status >/dev/null 2>&1; then state=active; fi
fi
printf 'installed=%s\nstate=%s\n' "$installed" "$state"
"#;

pub fn collect(server_id: &str, session: &Session) -> AppResult<SecurityReport> {
    let identity = checked(session, IDENTITY_SCRIPT, "读取系统身份信息")?;
    let identity = values(&identity.stdout);
    let manager = identity
        .get("manager")
        .map(String::as_str)
        .unwrap_or("unknown")
        .to_string();
    let hostname = identity
        .get("hostname")
        .cloned()
        .unwrap_or_else(|| "未知主机".into());
    let os = identity
        .get("os")
        .cloned()
        .unwrap_or_else(|| "未知系统".into());
    let mut warnings = Vec::new();

    let (updates, mut update_warnings) = collect_updates(session, &manager)?;
    warnings.append(&mut update_warnings);
    let reboot = ssh::execute(session, REBOOT_SCRIPT)?;
    let reboot_values = values(&reboot.stdout);
    let reboot_state = reboot_values
        .get("required")
        .map(String::as_str)
        .unwrap_or("unknown");
    if reboot_state == "unknown" {
        warnings.push("无法确认更新后的重启需求，请人工检查".into());
    }

    let listening_output = ssh::execute(
        session,
        "if command -v ss >/dev/null 2>&1; then ss -H -lntup 2>&1; elif command -v netstat >/dev/null 2>&1; then netstat -lntup 2>&1; else exit 127; fi",
    )?;
    let listening_ports = if listening_output.exit_code == 0 {
        parse_listening_ports(&listening_output.stdout)
    } else {
        warnings.push(format!("无法读取监听端口：{}", detail(&listening_output)));
        Vec::new()
    };

    let firewall = collect_firewall(session, &mut warnings)?;
    let ssh_output = ssh::execute(session, "sshd -T 2>/dev/null")?;
    let ssh_source;
    let ssh_values = if ssh_output.exit_code == 0 && !ssh_output.stdout.trim().is_empty() {
        ssh_source = "sshd -T 有效配置";
        values(&ssh_output.stdout)
    } else {
        ssh_source = "主配置文件静态读取";
        warnings.push(
            "无法运行 sshd -T，SSH 审计使用主配置文件与默认值，Include 和条件配置可能未覆盖".into(),
        );
        let fallback = ssh::execute(session, SSH_CONFIG_FALLBACK)?;
        values(&fallback.stdout)
    };

    let fail2ban_output = ssh::execute(session, FAIL2BAN_SCRIPT)?;
    let fail2ban = values(&fail2ban_output.stdout);
    let mut findings = ssh_findings(&ssh_values, ssh_source);
    findings.push(fail2ban_finding(&fail2ban));
    findings.push(firewall_finding(&firewall));
    let security_count = updates
        .iter()
        .filter(|update| update.kind == PackageUpdateKind::Security)
        .count();
    findings.push(SecurityFinding {
        id: "security-updates".into(),
        title: "安全补丁".into(),
        severity: if security_count > 0 {
            SecuritySeverity::Warning
        } else {
            SecuritySeverity::Pass
        },
        value: format!("{security_count} 项待安装"),
        detail: format!("基于 {} 的现有软件源缓存盘点", manager),
        recommendation: if security_count > 0 {
            "生成安全更新 Runbook，先灰度一台并确认健康检查后再扩大范围".into()
        } else {
            "现有缓存中未发现安全更新；仍需按维护窗口刷新软件源复核".into()
        },
        remediation_command: None,
    });
    if reboot_state == "yes" {
        findings.push(SecurityFinding {
            id: "reboot-required".into(),
            title: "系统重启需求".into(),
            severity: SecuritySeverity::Warning,
            value: "需要重启".into(),
            detail: reboot_values.get("reason").cloned().unwrap_or_default(),
            recommendation: "确认业务可中断后，在维护窗口手动重启；更新 Runbook 不会自动重启"
                .into(),
            remediation_command: None,
        });
    }

    Ok(SecurityReport {
        server_id: server_id.to_string(),
        hostname,
        os,
        package_manager: manager,
        updates,
        reboot_required: reboot_state == "yes",
        reboot_reason: reboot_values.get("reason").cloned().unwrap_or_default(),
        listening_ports,
        firewall,
        findings,
        warnings,
        collected_at: Utc::now().to_rfc3339(),
    })
}

fn collect_updates(
    session: &Session,
    manager: &str,
) -> AppResult<(Vec<PackageUpdate>, Vec<String>)> {
    let mut warnings = Vec::new();
    let command = match manager {
        "apt" => "apt list --upgradable 2>/dev/null",
        "dnf" => "dnf -q --cacheonly check-update 2>&1",
        "yum" => "yum -q --cacheonly check-update 2>&1",
        "apk" => "apk list -u 2>&1",
        _ => {
            warnings.push("未识别 apt、dnf、yum 或 apk，已跳过补丁盘点".into());
            return Ok((Vec::new(), warnings));
        }
    };
    let output = ssh::execute(session, command)?;
    if output.exit_code != 0 && !((manager == "dnf" || manager == "yum") && output.exit_code == 100)
    {
        warnings.push(format!("补丁列表读取失败：{}", detail(&output)));
        return Ok((Vec::new(), warnings));
    }

    let mut updates = match manager {
        "apt" => parse_apt_updates(&output.stdout),
        "dnf" | "yum" => {
            let installed = ssh::execute(
                session,
                "rpm -qa --qf '%{NAME}\\t%{VERSION}-%{RELEASE}\\n' 2>/dev/null",
            )?;
            let versions = installed
                .stdout
                .lines()
                .filter_map(|line| line.split_once('\t'))
                .map(|(name, version)| (name.to_string(), version.to_string()))
                .collect::<HashMap<_, _>>();
            let security = ssh::execute(
                session,
                &format!("{manager} -q --cacheonly check-update --security 2>/dev/null"),
            )?;
            let security_names = if security.exit_code == 0 || security.exit_code == 100 {
                parse_rpm_updates(&security.stdout)
                    .into_iter()
                    .map(|update| update.name)
                    .collect::<HashSet<_>>()
            } else {
                warnings.push("无法单独确认 RPM 安全更新，已仅按软件源名称辅助分类".into());
                HashSet::new()
            };
            let mut updates = parse_rpm_updates(&output.stdout);
            for update in &mut updates {
                update.current_version = versions
                    .get(&update.name)
                    .cloned()
                    .unwrap_or_else(|| "未知".into());
                if security_names.contains(&update.name) {
                    update.kind = PackageUpdateKind::Security;
                }
            }
            updates
        }
        "apk" => parse_apk_updates(&output.stdout),
        _ => Vec::new(),
    };
    updates.sort_by(|left, right| {
        right
            .kind
            .eq(&PackageUpdateKind::Security)
            .cmp(&left.kind.eq(&PackageUpdateKind::Security))
            .then_with(|| left.name.cmp(&right.name))
    });
    if updates.is_empty() {
        warnings.push("当前缓存未列出更新；这不等同于已刷新软件源后的最新结果".into());
    }
    Ok((updates, warnings))
}

fn parse_apt_updates(output: &str) -> Vec<PackageUpdate> {
    output
        .lines()
        .filter_map(|line| {
            let (package_source, rest) = line.trim().split_once(' ')?;
            let (name, source) = package_source.split_once('/')?;
            if name.is_empty() || !rest.contains("[upgradable from:") {
                return None;
            }
            let candidate = rest.split_whitespace().next()?.to_string();
            let current = rest
                .split_once("[upgradable from: ")?
                .1
                .trim_end_matches(']')
                .to_string();
            Some(package_update(name, &current, &candidate, source))
        })
        .collect()
}

fn parse_rpm_updates(output: &str) -> Vec<PackageUpdate> {
    output
        .lines()
        .filter_map(|line| {
            let columns = line.split_whitespace().collect::<Vec<_>>();
            if columns.len() < 3
                || line.starts_with("Last metadata")
                || line.starts_with("Obsoleting")
            {
                return None;
            }
            let package = columns[0];
            let name = package.split_once('.').map_or(package, |(name, _)| name);
            if columns[1].contains(':') && columns[1].split(':').next()?.parse::<u64>().is_err() {
                return None;
            }
            Some(package_update(name, "未知", columns[1], columns[2]))
        })
        .collect()
}

fn parse_apk_updates(output: &str) -> Vec<PackageUpdate> {
    output
        .lines()
        .filter_map(|line| {
            let columns = line.split_whitespace().collect::<Vec<_>>();
            if columns.len() < 4 {
                return None;
            }
            let (name, candidate) = split_apk_package(columns[0])?;
            let current = line
                .split_once("[upgradable from: ")?
                .1
                .trim_end_matches(']');
            let (_, current) = split_apk_package(current)?;
            let source = columns
                .iter()
                .find(|column| column.starts_with('{'))
                .map_or("apk", |value| value.trim_matches(&['{', '}'][..]));
            Some(package_update(name, &current, &candidate, source))
        })
        .collect()
}

fn split_apk_package(value: &str) -> Option<(&str, &str)> {
    let mut parts = value.rsplitn(3, '-');
    let release = parts.next()?;
    let version = parts.next()?;
    let name = parts.next()?;
    if name.is_empty() || version.is_empty() || release.is_empty() {
        return None;
    }
    Some((name, &value[name.len() + 1..]))
}

fn package_update(
    name: &str,
    current_version: &str,
    candidate_version: &str,
    source: &str,
) -> PackageUpdate {
    let lower_name = name.to_ascii_lowercase();
    let lower_source = source.to_ascii_lowercase();
    let kernel = lower_name == "kernel"
        || lower_name.starts_with("kernel-")
        || lower_name.starts_with("linux-image")
        || lower_name.starts_with("linux-headers")
        || lower_name.starts_with("linux-modules")
        || lower_name.starts_with("linux-generic")
        || lower_name.starts_with("linux-kernel");
    PackageUpdate {
        name: name.to_string(),
        current_version: current_version.to_string(),
        candidate_version: candidate_version.to_string(),
        source: source.to_string(),
        kind: if lower_source.contains("security") {
            PackageUpdateKind::Security
        } else {
            PackageUpdateKind::Normal
        },
        kernel,
    }
}

fn parse_listening_ports(output: &str) -> Vec<ListeningPort> {
    let mut ports = output
        .lines()
        .filter_map(|line| {
            let columns = line.split_whitespace().collect::<Vec<_>>();
            if columns.len() < 5 {
                return None;
            }
            let protocol = columns[0].to_ascii_lowercase();
            if protocol != "tcp" && protocol != "udp" {
                return None;
            }
            let local = columns.iter().skip(3).find(|value| {
                value
                    .rsplit_once(':')
                    .is_some_and(|(_, port)| !port.contains('*') && port.parse::<u16>().is_ok())
            })?;
            let (address, port) = local.rsplit_once(':')?;
            let port = port.parse::<u16>().ok()?;
            let address = address.trim_matches(&['[', ']'][..]).to_string();
            let process = line
                .split_once("users:((")
                .and_then(|(_, value)| value.split('"').nth(1))
                .unwrap_or("未知进程")
                .to_string();
            let exposure = if address == "0.0.0.0" || address == "::" || address == "*" {
                "all_interfaces"
            } else if address == "127.0.0.1"
                || address == "::1"
                || address.starts_with("127.")
                || address == "localhost"
            {
                "localhost"
            } else {
                "external_interface"
            };
            Some(ListeningPort {
                protocol,
                bind_address: address,
                port,
                process,
                exposure: exposure.to_string(),
            })
        })
        .collect::<Vec<_>>();
    ports.sort_by(|left, right| {
        left.port
            .cmp(&right.port)
            .then_with(|| left.protocol.cmp(&right.protocol))
    });
    ports.dedup_by(|left, right| {
        left.protocol == right.protocol
            && left.bind_address == right.bind_address
            && left.port == right.port
            && left.process == right.process
    });
    ports
}

fn collect_firewall(session: &Session, warnings: &mut Vec<String>) -> AppResult<FirewallStatus> {
    let mut fallback = None;
    let ufw = ssh::execute(
        session,
        "if command -v ufw >/dev/null 2>&1; then ufw status verbose 2>&1; else exit 127; fi",
    )?;
    if ufw.exit_code == 0 {
        let active = ufw.stdout.to_ascii_lowercase().contains("status: active");
        let status = FirewallStatus {
            backend: "ufw".into(),
            state: if active { "active" } else { "inactive" }.into(),
            active,
            details: ufw.stdout.trim().to_string(),
        };
        if active {
            return Ok(status);
        }
        fallback = Some(status);
    } else if ufw.exit_code != 127 {
        warnings.push(format!("ufw 状态读取失败：{}", detail(&ufw)));
    }

    let firewalld = ssh::execute(
        session,
        "if command -v firewall-cmd >/dev/null 2>&1; then firewall-cmd --state 2>&1 && firewall-cmd --list-all 2>&1; else exit 127; fi",
    )?;
    if firewalld.exit_code == 0 && firewalld.stdout.contains("running") {
        return Ok(FirewallStatus {
            backend: "firewalld".into(),
            state: "active".into(),
            active: true,
            details: firewalld.stdout.trim().to_string(),
        });
    }
    if firewalld.exit_code != 127 && firewalld.exit_code != 252 {
        warnings.push(format!("firewalld 状态读取失败：{}", detail(&firewalld)));
    }

    let nft = ssh::execute(
        session,
        "if command -v nft >/dev/null 2>&1; then nft list ruleset 2>&1; else exit 127; fi",
    )?;
    if nft.exit_code == 0 {
        let active = !nft.stdout.trim().is_empty();
        let status = FirewallStatus {
            backend: "nftables".into(),
            state: if active { "ruleset_present" } else { "empty" }.into(),
            active,
            details: nft.stdout.trim().to_string(),
        };
        if active {
            return Ok(status);
        }
        if fallback.is_none() {
            fallback = Some(status);
        }
    } else if nft.exit_code != 127 {
        warnings.push(format!("nftables 状态读取失败：{}", detail(&nft)));
    }
    Ok(fallback.unwrap_or(FirewallStatus {
        backend: "未识别".into(),
        state: "not_detected".into(),
        active: false,
        details: "未检测到 ufw、firewalld 或可读取的 nftables 规则".into(),
    }))
}

fn ssh_findings(values: &HashMap<String, String>, source: &str) -> Vec<SecurityFinding> {
    let get = |key: &str, default: &str| {
        values
            .get(key)
            .map(String::as_str)
            .unwrap_or(default)
            .to_ascii_lowercase()
    };
    let password = get("passwordauthentication", "yes");
    let root = get("permitrootlogin", "prohibit-password");
    let pubkey = get("pubkeyauthentication", "yes");
    let empty = get("permitemptypasswords", "no");
    let forwarding = get("allowtcpforwarding", "yes");
    let x11 = get("x11forwarding", "no");
    let max_auth = get("maxauthtries", "6");
    vec![
        SecurityFinding {
            id: "ssh-password".into(),
            title: "SSH 密码登录".into(),
            severity: if password == "yes" {
                SecuritySeverity::Warning
            } else {
                SecuritySeverity::Pass
            },
            value: password.clone(),
            detail: format!("来源：{source}"),
            recommendation: if password == "yes" {
                "先确认密钥登录和备用会话可用，再人工审查禁用密码登录的命令".into()
            } else {
                "密码登录已关闭".into()
            },
            remediation_command: (password == "yes")
                .then(|| ssh_remediation("PasswordAuthentication no")),
        },
        SecurityFinding {
            id: "ssh-root".into(),
            title: "SSH root 登录".into(),
            severity: if root == "yes" {
                SecuritySeverity::Critical
            } else {
                SecuritySeverity::Pass
            },
            value: root.clone(),
            detail: format!("来源：{source}"),
            recommendation: if root == "yes" {
                "建议普通用户 sudo，并将 root 限制为仅密钥登录；修改前保留已验证的备用会话".into()
            } else {
                "root 未开放密码登录".into()
            },
            remediation_command: (root == "yes")
                .then(|| ssh_remediation("PermitRootLogin prohibit-password")),
        },
        SecurityFinding {
            id: "ssh-pubkey".into(),
            title: "SSH 公钥认证".into(),
            severity: if pubkey == "no" {
                SecuritySeverity::Warning
            } else {
                SecuritySeverity::Pass
            },
            value: pubkey,
            detail: format!("来源：{source}"),
            recommendation: "为自动化与管理员配置独立密钥，避免仅依赖密码".into(),
            remediation_command: None,
        },
        SecurityFinding {
            id: "ssh-empty-password".into(),
            title: "SSH 空密码".into(),
            severity: if empty == "yes" {
                SecuritySeverity::Critical
            } else {
                SecuritySeverity::Pass
            },
            value: empty,
            detail: format!("来源：{source}"),
            recommendation: "空密码认证应始终关闭，并检查系统是否存在空密码账户".into(),
            remediation_command: None,
        },
        SecurityFinding {
            id: "ssh-forwarding".into(),
            title: "SSH 转发".into(),
            severity: if forwarding == "no" {
                SecuritySeverity::Pass
            } else {
                SecuritySeverity::Info
            },
            value: forwarding,
            detail: format!("来源：{source}；ControlVPS 隧道功能依赖此能力"),
            recommendation: "按账号与用途限制转发，不要在共享堡垒账号上无条件开放".into(),
            remediation_command: None,
        },
        SecurityFinding {
            id: "ssh-x11".into(),
            title: "X11 转发".into(),
            severity: if x11 == "yes" {
                SecuritySeverity::Info
            } else {
                SecuritySeverity::Pass
            },
            value: x11,
            detail: format!("来源：{source}"),
            recommendation: "纯服务器无需桌面转发时可人工评估关闭".into(),
            remediation_command: None,
        },
        SecurityFinding {
            id: "ssh-max-auth".into(),
            title: "SSH 认证尝试次数".into(),
            severity: if max_auth.parse::<u32>().unwrap_or(6) > 6 {
                SecuritySeverity::Warning
            } else {
                SecuritySeverity::Pass
            },
            value: max_auth,
            detail: format!("来源：{source}"),
            recommendation: "建议 MaxAuthTries 不超过 6，并结合 Fail2ban 限制暴力尝试".into(),
            remediation_command: None,
        },
    ]
}

fn fail2ban_finding(values: &HashMap<String, String>) -> SecurityFinding {
    let installed = values.get("installed").is_some_and(|value| value == "yes");
    let active = values.get("state").is_some_and(|value| value == "active");
    SecurityFinding {
        id: "fail2ban".into(),
        title: "Fail2ban 暴力破解防护".into(),
        severity: if active {
            SecuritySeverity::Pass
        } else {
            SecuritySeverity::Warning
        },
        value: if active {
            "运行中"
        } else if installed {
            "已安装但未运行"
        } else {
            "未安装"
        }
        .into(),
        detail: "只读取服务状态，不修改防火墙或封禁规则".into(),
        recommendation: "结合日志来源与业务访问方式配置 SSH jail，避免误封管理地址".into(),
        remediation_command: None,
    }
}

fn firewall_finding(firewall: &FirewallStatus) -> SecurityFinding {
    SecurityFinding {
        id: "firewall".into(),
        title: "主机防火墙".into(),
        severity: if firewall.active {
            SecuritySeverity::Pass
        } else {
            SecuritySeverity::Warning
        },
        value: format!("{} · {}", firewall.backend, firewall.state),
        detail: "对外监听不代表公网一定可达，云安全组、上游 NAT 与主机防火墙需结合判断".into(),
        recommendation: if firewall.active {
            "核对放通端口是否均为业务必需".into()
        } else {
            "若云平台安全组未限制入站，请配置主机防火墙并先保留当前 SSH 管理端口".into()
        },
        remediation_command: None,
    }
}

fn ssh_remediation(directive: &str) -> String {
    let file = if directive.starts_with("PasswordAuthentication") {
        "60-controlvps-password.conf"
    } else {
        "60-controlvps-root.conf"
    };
    format!(
        "install -d -m 700 /etc/ssh/sshd_config.d\nprintf '%s\\n' '{directive}' > /etc/ssh/sshd_config.d/{file}\nif sshd -t; then systemctl reload sshd 2>/dev/null || systemctl reload ssh; else rm -f /etc/ssh/sshd_config.d/{file}; exit 1; fi"
    )
}

fn checked(session: &Session, command: &str, action: &str) -> AppResult<ExecOutput> {
    let output = ssh::execute(session, command)?;
    if output.exit_code != 0 {
        return Err(AppError::Message(format!(
            "{action}失败：{}",
            detail(&output)
        )));
    }
    Ok(output)
}

fn values(output: &str) -> HashMap<String, String> {
    output
        .lines()
        .filter_map(|line| {
            line.split_once('=')
                .or_else(|| line.split_once(char::is_whitespace))
        })
        .map(|(key, value)| (key.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect()
}

fn detail(output: &ExecOutput) -> String {
    let text = if output.stderr.trim().is_empty() {
        output.stdout.trim()
    } else {
        output.stderr.trim()
    };
    text.chars().take(500).collect()
}
