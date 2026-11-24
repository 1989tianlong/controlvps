import { lazy, Suspense, useState } from "react";
import { Activity, Clock3, HardDrive, List, Power, RefreshCw, ScrollText, Server as ServerIcon, Trash2 } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import { formatBytes, formatTime, formatUptime, percent } from "../lib/format";
import type { ActionResult, QuickAction, Server, ServerMetrics, TrustedExecutor } from "../types";
import { FileManager } from "./FileManager";
import { DockerManager } from "./DockerManager";
import { SystemdManager } from "./SystemdManager";

const TerminalView = lazy(() => import("./TerminalView").then((module) => ({ default: module.TerminalView })));

export type DetailTab = "overview" | "terminal" | "files" | "docker" | "services" | "actions";

interface Props {
  server: Server;
  metrics?: ServerMetrics;
  metricsError?: string;
  tab: DetailTab;
  onTabChange: (tab: DetailTab) => void;
  onRefresh: () => void;
  refreshing: boolean;
  trusted: TrustedExecutor;
}

export function ServerDetail({ server, metrics, metricsError, tab, onTabChange, onRefresh, refreshing, trusted }: Props) {
  return (
    <section className="panel server-detail" aria-label={`${server.name} 详情`}>
      <div className="panel-header">
        <div><h2>{server.name}</h2><span className="table-meta">{server.username}@{server.host}:{server.port}</span></div>
        <button className="btn" type="button" onClick={onRefresh} disabled={refreshing} aria-busy={refreshing}><RefreshCw size={15} />{refreshing ? "正在采集" : "刷新指标"}</button>
      </div>
      <div className="tabs" role="tablist" aria-label="服务器工作区">
        <button className="tab" type="button" role="tab" aria-selected={tab === "overview"} onClick={() => onTabChange("overview")}>概览</button>
        <button className="tab" type="button" role="tab" aria-selected={tab === "terminal"} onClick={() => onTabChange("terminal")}>SSH 终端</button>
        <button className="tab" type="button" role="tab" aria-selected={tab === "files"} onClick={() => onTabChange("files")}>文件管理</button>
        <button className="tab" type="button" role="tab" aria-selected={tab === "docker"} onClick={() => onTabChange("docker")}>Docker</button>
        <button className="tab" type="button" role="tab" aria-selected={tab === "services"} onClick={() => onTabChange("services")}>服务管理</button>
        <button className="tab" type="button" role="tab" aria-selected={tab === "actions"} onClick={() => onTabChange("actions")}>快捷运维</button>
      </div>
      {tab === "overview" && <OverviewTab server={server} metrics={metrics} error={metricsError} onRefresh={onRefresh} refreshing={refreshing} />}
      {tab === "terminal" && <Suspense fallback={<div className="terminal-empty">正在加载终端组件</div>}><TerminalView server={server} trusted={trusted} /></Suspense>}
      {tab === "files" && <FileManager server={server} trusted={trusted} />}
      {tab === "docker" && <DockerManager server={server} trusted={trusted} />}
      {tab === "services" && <SystemdManager server={server} trusted={trusted} />}
      {tab === "actions" && <QuickActions server={server} trusted={trusted} />}
    </section>
  );
}

function OverviewTab({ server, metrics, error, onRefresh, refreshing }: { server: Server; metrics?: ServerMetrics; error?: string; onRefresh: () => void; refreshing: boolean }) {
  if (!metrics) {
    return <div className="empty"><div><Activity size={30} /><h2>尚未采集运行指标</h2><p>{error ?? "通过 SSH 读取 Linux 系统指标，不需要安装 Agent。"}</p><button className="btn btn-primary" type="button" onClick={onRefresh} disabled={refreshing}>立即检查</button></div></div>;
  }
  const values = [
    { label: "CPU", value: `${metrics.cpuUsage.toFixed(1)}%`, note: `负载 ${metrics.loadAverage.map((value) => value.toFixed(2)).join(" / ")}`, used: metrics.cpuUsage },
    { label: "内存", value: `${percent(metrics.memoryUsed, metrics.memoryTotal).toFixed(1)}%`, note: `${formatBytes(metrics.memoryUsed)} / ${formatBytes(metrics.memoryTotal)}`, used: percent(metrics.memoryUsed, metrics.memoryTotal) },
    { label: "系统盘", value: `${percent(metrics.diskUsed, metrics.diskTotal).toFixed(1)}%`, note: `${formatBytes(metrics.diskUsed)} / ${formatBytes(metrics.diskTotal)}`, used: percent(metrics.diskUsed, metrics.diskTotal) },
    { label: "网络", value: formatBytes(metrics.networkRxBytesPerSecond, true), note: `上行 ${formatBytes(metrics.networkTxBytesPerSecond, true)}`, used: null },
  ];
  return <div className="panel-body">
    {error && <div className="error-banner" role="alert">{error}</div>}
    <div className="metric-grid">
      {values.map((item) => <div className="metric" key={item.label}><small>{item.label}</small><strong className="metric-value">{item.value}</strong><small>{item.note}</small>{item.used !== null && <progress max={100} value={item.used} aria-label={`${item.label} 使用率`} />}</div>)}
    </div>
    <dl className="facts">
      <div><dt>主机名</dt><dd>{metrics.hostname}</dd></div>
      <div><dt>操作系统</dt><dd>{metrics.os}</dd></div>
      <div><dt>运行时间</dt><dd>{formatUptime(metrics.uptimeSeconds)}</dd></div>
      <div><dt>采集时间</dt><dd>{formatTime(metrics.collectedAt)}</dd></div>
      <div><dt>服务商与地区</dt><dd>{[server.provider, server.region].filter(Boolean).join("，") || "未填写"}</dd></div>
      <div><dt>资产备注</dt><dd>{server.notes || "无备注"}</dd></div>
    </dl>
  </div>;
}

const actions: { id: QuickAction; name: string; description: string; icon: typeof ServerIcon }[] = [
  { id: "system_info", name: "系统信息", description: "内核、系统、CPU 与内存", icon: ServerIcon },
  { id: "processes", name: "高占用进程", description: "按 CPU 排序的进程列表", icon: List },
  { id: "disk_usage", name: "磁盘与 inode", description: "所有已挂载文件系统", icon: HardDrive },
  { id: "system_logs", name: "最近系统日志", description: "读取最近 80 行日志", icon: ScrollText },
  { id: "clean_cache", name: "清理包缓存", description: "需要相应系统权限", icon: Trash2 },
  { id: "reboot", name: "重启服务器", description: "会中断所有运行服务", icon: Power },
];

function QuickActions({ server, trusted }: { server: Server; trusted: TrustedExecutor }) {
  const [busy, setBusy] = useState<QuickAction | null>(null);
  const [result, setResult] = useState<ActionResult | null>(null);
  const [error, setError] = useState("");
  const run = async (action: QuickAction) => {
    if (action === "reboot" && !window.confirm(`确认重启 ${server.name}？运行中的服务会中断。`)) return;
    if (action === "clean_cache" && !window.confirm(`确认清理 ${server.name} 的包管理器缓存？`)) return;
    setBusy(action);
    setError("");
    try {
      setResult(await trusted(server, (allow) => api.quickAction(server.id, action, allow)));
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(null);
    }
  };
  return <div className="panel-body">
    {error && <div className="error-banner" role="alert">{error}</div>}
    <div className="action-list">
      {actions.map((action) => <button className="btn justify-start" type="button" key={action.id} title={action.description} onClick={() => run(action.id)} disabled={busy !== null} aria-busy={busy === action.id}><action.icon size={16} />{busy === action.id ? "正在执行" : action.name}</button>)}
    </div>
    {result && <div className="mt-6"><div className="flex items-center justify-between gap-4"><span className={result.exitCode === 0 ? "badge badge-ok" : "badge badge-bad"}>退出码 {result.exitCode}</span><span className="measured">{result.durationMs} ms</span></div><pre className="action-output"><code>{result.output || "命令没有输出"}</code></pre></div>}
    {!result && <p className="mt-6 flex items-center gap-2 text-muted text-sm"><Clock3 size={15} />操作结果只保留在当前页面，审计记录保存动作、状态与耗时。</p>}
  </div>;
}
