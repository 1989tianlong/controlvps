import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AlertTriangle, CheckCircle2, ChevronRight, CircleAlert, FileCheck2, Network, Info, PackageCheck, RefreshCw, Search, Server as ServerIcon, ShieldAlert, ShieldCheck, TerminalSquare } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import { formatTime } from "../lib/format";
import type { PackageUpdate, RunbookInput, SecurityFinding, SecurityReport, Server, TrustedExecutor } from "../types";

interface Props {
  servers: Server[];
  trusted: TrustedExecutor;
  onAdd: () => void;
  onOpenRunbooks: () => void;
}

type Section = "patches" | "exposure" | "hardening";
type PatchFilter = "all" | "security" | "kernel";

export function SecurityCenter({ servers, trusted, onAdd, onOpenRunbooks }: Props) {
  const [selectedId, setSelectedId] = useState("");
  const [report, setReport] = useState<SecurityReport>();
  const [section, setSection] = useState<Section>("patches");
  const [filter, setFilter] = useState<PatchFilter>("all");
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState("");
  const server = servers.find((item) => item.id === selectedId) ?? servers[0];
  const serverRef = useRef(server);
  serverRef.current = server;
  const serverId = server?.id ?? "";
  const requestIdRef = useRef(0);

  const load = useCallback(async () => {
    const target = serverRef.current;
    if (!target) return;
    const requestId = ++requestIdRef.current;
    setBusy(true);
    setError("");
    try {
      const next = await trusted(target, (allow) => api.collectSecurityReport(target.id, allow));
      if (requestIdRef.current !== requestId) return;
      setReport(next);
    } catch (reason) {
      if (requestIdRef.current !== requestId) return;
      setReport(undefined);
      setError(errorMessage(reason));
    } finally {
      if (requestIdRef.current === requestId) setBusy(false);
    }
  }, [serverId, trusted]);

  useEffect(() => {
    requestIdRef.current += 1;
    setReport(undefined);
    setError("");
    if (serverId) void load();
  }, [serverId, load]);

  const securityCount = report?.updates.filter((update) => update.kind === "security").length ?? 0;
  const kernelCount = report?.updates.filter((update) => update.kernel).length ?? 0;
  const externalCount = report?.listeningPorts.filter((port) => port.exposure !== "localhost").length ?? 0;
  const riskCount = report?.findings.filter((finding) => finding.severity === "warning" || finding.severity === "critical").length ?? 0;
  const updates = useMemo(() => report?.updates.filter((update) => {
    const matchesFilter = filter === "all" || (filter === "security" ? update.kind === "security" : update.kernel);
    return matchesFilter && `${update.name} ${update.source}`.toLowerCase().includes(query.toLowerCase());
  }) ?? [], [report, filter, query]);

  const createPatchRunbook = async (securityOnly: boolean) => {
    if (!server || !report) return;
    const selectedUpdates = report.updates.filter((update) => !securityOnly || update.kind === "security");
    if (selectedUpdates.length === 0) return;
    setCreating(true);
    setError("");
    try {
      const input = patchRunbook(server, report.packageManager, selectedUpdates, securityOnly);
      await api.createRunbook(input);
      onOpenRunbooks();
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setCreating(false); }
  };

  return <>
    <div className="page-heading"><div><h1>补丁与安全</h1><p>只读盘点系统更新、对外监听与 SSH 配置，再把更新转为可审查的灰度 Runbook。</p></div><div className="heading-actions"><select className="select" aria-label="安全审计服务器" value={server?.id ?? ""} disabled={busy || creating || servers.length === 0} onChange={(event) => setSelectedId(event.target.value)}>{servers.map((item) => <option value={item.id} key={item.id}>{item.name} · {item.host}</option>)}</select><button className="btn btn-primary" type="button" disabled={!server || busy || creating} onClick={() => void load()}><RefreshCw size={16} />{busy ? "正在审计" : "重新扫描"}</button></div></div>
    <div className="risk security-readonly"><ShieldCheck size={20} /><p><strong>扫描不会刷新软件源、安装补丁或修改配置。</strong>端口“对外监听”不等于公网可达，还需结合云安全组和上游防火墙判断。</p></div>
    {error && <div className="error-banner" role="alert"><p>{error}</p><button className="btn" type="button" onClick={() => void load()}>重试</button></div>}
    {!server && <div className="empty"><div><ServerIcon size={30} /><h2>先添加 VPS</h2><p>通过现有 SSH 连接进行无 Agent 安全审计。</p><button className="btn btn-primary" type="button" onClick={onAdd}>添加 VPS</button></div></div>}
    {server && busy && !report && <div className="security-loading"><div className="skeleton" /><div className="skeleton" /><div className="skeleton" /></div>}
    {report && <>
      <section className="panel security-target"><div><div className="detail-title"><ShieldCheck size={21} /><h2>{report.hostname}</h2><span className={report.rebootRequired ? "badge badge-bad" : "badge badge-ok"}>{report.rebootRequired ? "需要重启" : "未报告重启需求"}</span></div><p>{report.os} · {report.packageManager.toUpperCase()} · {formatTime(report.collectedAt)}</p></div><span className="badge badge-cool"><FileCheck2 size={15} />只读审计</span></section>
      {report.warnings.length > 0 && <details className="security-warnings"><summary><AlertTriangle size={16} />{report.warnings.length} 条扫描提示<ChevronRight size={16} /></summary><ul>{report.warnings.map((warning, index) => <li key={index}>{warning}</li>)}</ul></details>}
      <div className="stats security-stats"><div className="stat"><span className="stat-label">安全更新</span><strong className="stat-value">{securityCount}</strong><span className="stat-note">共 {report.updates.length} 项可更新</span></div><div className="stat"><span className="stat-label">内核相关</span><strong className="stat-value">{kernelCount}</strong><span className="stat-note">{report.rebootRequired ? report.rebootReason : "重启前需人工确认业务窗口"}</span></div><div className="stat"><span className="stat-label">对外监听</span><strong className="stat-value">{externalCount}</strong><span className="stat-note">共 {report.listeningPorts.length} 个 TCP / UDP 监听</span></div><div className="stat"><span className="stat-label">风险与建议</span><strong className="stat-value">{riskCount}</strong><span className="stat-note">{report.firewall.backend} · {report.firewall.active ? "防火墙活动" : "未检测到活动防火墙"}</span></div></div>
      <div className="workspace-switch" role="tablist" aria-label="安全中心视图"><button className="tab" type="button" role="tab" aria-selected={section === "patches"} onClick={() => setSection("patches")}><PackageCheck size={16} />系统补丁</button><button className="tab" type="button" role="tab" aria-selected={section === "exposure"} onClick={() => setSection("exposure")}><Network size={16} />端口与防火墙</button><button className="tab" type="button" role="tab" aria-selected={section === "hardening"} onClick={() => setSection("hardening")}><ShieldAlert size={16} />SSH 与加固</button></div>
      {section === "patches" && <section className="panel security-panel"><div className="panel-header"><div><h2>可安装更新</h2><span className="table-meta">版本变化来自服务器现有包缓存，生成后仍需审查命令</span></div><div className="heading-actions"><button className="btn" type="button" disabled={creating || securityCount === 0 || report.packageManager === "unknown"} onClick={() => void createPatchRunbook(true)}><ShieldCheck size={15} />{creating ? "正在生成" : "生成安全更新 Runbook"}</button><button className="btn btn-primary" type="button" disabled={creating || report.updates.length === 0 || report.packageManager === "unknown"} onClick={() => void createPatchRunbook(false)}><PackageCheck size={15} />生成全部更新 Runbook</button></div></div><div className="security-patch-filters"><div className="search-wrap"><Search size={16} /><input className="input" type="search" aria-label="搜索补丁" placeholder="搜索软件包或软件源" value={query} onChange={(event) => setQuery(event.target.value)} /></div><select className="select" aria-label="补丁类型" value={filter} onChange={(event) => setFilter(event.target.value as PatchFilter)}><option value="all">全部更新</option><option value="security">仅安全更新</option><option value="kernel">仅内核相关</option></select></div>{updates.length > 0 ? <div className="table-wrap"><table className="data-table security-patch-table"><thead><tr><th>软件包</th><th>当前版本</th><th>候选版本</th><th>类型 / 来源</th></tr></thead><tbody>{updates.map((update) => <tr key={`${update.name}-${update.candidateVersion}`}><td><span className="table-name measured">{update.name}</span></td><td className="measured security-version">{update.currentVersion}</td><td className="measured security-version security-candidate">{update.candidateVersion}</td><td><span className={update.kind === "security" ? "badge badge-bad" : "badge"}>{update.kind === "security" ? "安全" : "普通"}</span>{update.kernel && <span className="badge badge-cool security-kind">内核</span>}<span className="table-meta">{update.source}</span></td></tr>)}</tbody></table></div> : <div className="security-empty"><PackageCheck size={28} /><h3>{report.updates.length === 0 ? "当前缓存没有列出更新" : "没有匹配的更新"}</h3><p>{report.updates.length === 0 ? "如需刷新软件源，请先在终端执行对应包管理器的元数据刷新命令，再重新扫描。" : "调整搜索词或补丁类型。"}</p></div>}</section>}
      {section === "exposure" && <div className="security-exposure"><section className="panel"><div className="panel-header"><div><h2>主机防火墙</h2><span className="table-meta">{report.firewall.backend} · {report.firewall.state}</span></div><span className={report.firewall.active ? "badge badge-ok" : "badge badge-bad"}>{report.firewall.active ? "活动" : "未确认活动"}</span></div><div className="panel-body"><p className="security-explanation">防火墙规则仅做读取。若当前 SSH 端口被误封，可能立即失联，因此这里不提供一键修改。</p><details><summary>查看规则与状态详情</summary><pre className="action-output"><code>{report.firewall.details || "没有规则输出"}</code></pre></details></div></section><section className="panel"><div className="panel-header"><div><h2>监听端口</h2><span className="table-meta">按监听地址初步判断暴露范围</span></div><span className="badge badge-cool">{externalCount} 个非回环监听</span></div>{report.listeningPorts.length > 0 ? <div className="table-wrap"><table className="data-table"><thead><tr><th>协议 / 端口</th><th>监听地址</th><th>进程</th><th>范围</th></tr></thead><tbody>{report.listeningPorts.map((port) => <tr key={`${port.protocol}-${port.bindAddress}-${port.port}-${port.process}`}><td><span className="table-name measured">{port.protocol.toUpperCase()} / {port.port}</span></td><td className="measured">{port.bindAddress}</td><td>{port.process}</td><td><span className={port.exposure === "localhost" ? "badge badge-ok" : "badge badge-bad"}>{port.exposure === "localhost" ? "仅本机" : port.exposure === "all_interfaces" ? "所有网卡" : "非回环网卡"}</span></td></tr>)}</tbody></table></div> : <div className="security-empty"><Info size={26} /><p>未读取到监听端口，请检查扫描提示和系统权限。</p></div>}</section></div>}
      {section === "hardening" && <div className="security-findings">{report.findings.map((finding) => <FindingCard finding={finding} key={finding.id} />)}</div>}
    </>}
  </>;
}

function FindingCard({ finding }: { finding: SecurityFinding }) {
  const Icon = finding.severity === "pass" ? CheckCircle2 : finding.severity === "info" ? Info : finding.severity === "critical" ? ShieldAlert : CircleAlert;
  const label = finding.severity === "pass" ? "通过" : finding.severity === "info" ? "关注" : finding.severity === "critical" ? "高风险" : "建议修复";
  return <article className="panel security-finding" data-severity={finding.severity}><div className="security-finding-icon"><Icon size={21} /></div><div className="security-finding-content"><div className="security-finding-heading"><h2>{finding.title}</h2><span className={finding.severity === "pass" ? "badge badge-ok" : finding.severity === "info" ? "badge badge-cool" : "badge badge-bad"}>{label}</span><span className="measured">{finding.value}</span></div><p>{finding.detail}</p><p className="security-recommendation">{finding.recommendation}</p>{finding.remediationCommand && <details className="security-remediation"><summary><TerminalSquare size={15} />查看建议命令（不会自动执行）</summary><div className="risk"><ShieldAlert size={18} /><p>先确认主配置包含 sshd_config.d、密钥登录可用，并保留备用会话。命令需在人工审查后自行执行。</p></div><pre className="review-command"><code>{finding.remediationCommand}</code></pre></details>}</div></article>;
}

function patchRunbook(server: Server, manager: SecurityReport["packageManager"], updates: PackageUpdate[], securityOnly: boolean): RunbookInput {
  const names = updates.map((update) => shellQuote(update.name)).join(" ");
  const refresh = manager === "apt" ? "apt-get update" : manager === "dnf" ? "dnf makecache" : manager === "yum" ? "yum makecache" : "apk update";
  const install = manager === "apt" ? `apt-get install --only-upgrade -y -- ${names}` : manager === "apk" ? `apk upgrade ${names}` : `${manager} upgrade -y -- ${names}`;
  return {
    name: `${securityOnly ? "安全更新" : "系统更新"} · ${server.name}`,
    description: `由补丁中心生成，共 ${updates.length} 个软件包。建议首批灰度 1 台；不会自动重启。`,
    steps: [
      { name: "更新前预检", command: `set -eu\nprintf '更新目标：%s\\n' ${shellQuote(server.name)}\ndf -h /\navailable=$(df -Pk / | awk 'NR==2 {print $4}')\n[ "\${available:-0}" -ge 524288 ] || { echo '系统盘可用空间不足 512 MiB'; exit 1; }\nif command -v systemctl >/dev/null 2>&1; then systemctl --failed --no-pager --plain; fi` },
      { name: "刷新软件源元数据", command: refresh },
      { name: `安装${securityOnly ? "安全" : "已盘点"}更新（${updates.length} 项）`, command: install },
      { name: "更新后健康检查", command: `set -eu\nif command -v systemctl >/dev/null 2>&1; then\n  failed=$(systemctl --failed --no-legend --plain --no-pager | awk 'NF && $1 != "0"')\n  if [ -n "$failed" ]; then printf '%s\\n' "$failed"; echo '存在失败的 systemd 服务'; exit 1; fi\nfi\nprintf '更新后 SSH 会话与基础健康检查正常\\n'\nif command -v ss >/dev/null 2>&1; then ss -H -lnt; fi` },
      { name: "重启需求复核（不重启）", command: `if [ -r /var/run/reboot-required ]; then cat /var/run/reboot-required; elif command -v needs-restarting >/dev/null 2>&1; then needs-restarting -r || true; else echo '请人工确认内核或核心库更新是否需要重启'; fi` },
    ],
  };
}

function shellQuote(value: string) {
  return `'${value.replaceAll("'", `'"'"'`)}'`;
}
