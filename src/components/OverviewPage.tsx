import { useCallback, useEffect, useState } from "react";
import { ArrowRight, BellRing, CheckCircle2, Compass, FolderArchive, Plus, RefreshCw, Server as ServerIcon, ShieldAlert, ShieldCheck, Sparkles, TriangleAlert } from "lucide-react";
import { api } from "../lib/api";
import { formatTime } from "../lib/format";
import type { ActionCenterSummary, Server, ServerMetrics } from "../types";

interface Props {
  servers: Server[];
  loading: boolean;
  metrics: Record<string, ServerMetrics>;
  onAdd: () => void;
  onServers: () => void;
  onDiscover: () => void;
  onMonitoring: () => void;
  onAutomation: () => void;
  onSettings: () => void;
  onDiagnose?: (prompt: string, serverId?: string, contextNote?: string) => void;
}

function currencySymbol(currency: string) {
  switch (currency) {
    case "USD": return "$";
    case "EUR": return "€";
    case "CNY":
    case "RMB": return "¥";
    case "GBP": return "£";
    default: return `${currency} `;
  }
}

export function OverviewPage({
  servers,
  loading,
  metrics,
  onAdd,
  onServers,
  onDiscover,
  onMonitoring,
  onAutomation,
  onSettings,
  onDiagnose,
}: Props) {
  const [actionCenter, setActionCenter] = useState<ActionCenterSummary>();
  const [refreshingTodos, setRefreshingTodos] = useState(false);

  const loadActionCenter = useCallback(async () => {
    setRefreshingTodos(true);
    try {
      setActionCenter(await api.actionCenter());
    } catch {
      // 待办面板读取失败不阻塞总览
    } finally {
      setRefreshingTodos(false);
    }
  }, []);

  useEffect(() => {
    void loadActionCenter();
  }, [loadActionCenter, servers.length]);

  const groups = new Set(servers.map((server) => server.groupName).filter(Boolean)).size;
  const measured = servers.filter((server) => metrics[server.id]).length;
  const costEntries = new Map<string, { total: number; count: number }>();
  for (const server of servers) {
    if (typeof server.cost !== "number" || !Number.isFinite(server.cost)) continue;
    const currency = server.currency || "USD";
    const current = costEntries.get(currency) ?? { total: 0, count: 0 };
    costEntries.set(currency, { total: current.total + server.cost, count: current.count + 1 });
  }
  const expiring = servers.flatMap((server) => {
    if (!server.expiresAt) return [];
    const due = Date.parse(server.expiresAt.length <= 10 ? `${server.expiresAt}T00:00:00` : server.expiresAt);
    if (Number.isNaN(due)) return [];
    const days = Math.ceil((due - Date.now()) / 86_400_000);
    return days <= 30 ? [{ server, days }] : [];
  }).sort((a, b) => a.days - b.days);
  const unpriced = servers.filter((server) => typeof server.cost !== "number").length;

  const urgentExpiring = actionCenter?.expiringServers.filter((item) => item.daysRemaining <= 14) ?? [];
  const totalTodos = actionCenter
    ? actionCenter.activeHostAlerts.length
      + actionCenter.activeServiceEvents.length
      + actionCenter.expiringCertificates.length
      + actionCenter.failedBackups.length
      + actionCenter.failedRunbooks.length
      + urgentExpiring.length
    : 0;

  return <>
    <div className="page-heading"><div><h1>VPS 工作台</h1><p>从同一个本地工作区管理连接与运行状态。</p></div><div className="heading-actions"><button className="btn" type="button" onClick={onDiscover}><Compass size={16} />发现 VPS</button><button className="btn btn-primary" type="button" onClick={onAdd}><Plus size={16} />添加 VPS</button></div></div>
    <div className="stats">
      <div className="stat"><span className="stat-label">已保存服务器</span><strong className="stat-value">{servers.length}</strong><span className="stat-note">{groups} 个资产分组</span></div>
      <div className="stat"><span className="stat-label">今日待办</span><strong className="stat-value">{totalTodos}</strong><span className="stat-note">{totalTodos === 0 ? "当前全部健康" : "告警/到期/失败项"}</span></div>
      <div className="stat"><span className="stat-label">定时备份策略</span><strong className="stat-value">{actionCenter?.scheduledBackupPolicies ?? 0}</strong><span className="stat-note">{measured} 台已有监控读数</span></div>
      <div className="stat"><span className="stat-label">外部告警渠道</span><strong className="stat-value">{actionCenter?.enabledNotificationChannels ?? 0}</strong><span className="stat-note">凭据由系统钥匙串保护</span></div>
    </div>

    {actionCenter && <section className="panel mb-6" aria-label="今日运维待办">
      <div className="panel-header">
        <div className="flex items-center gap-3">
          {totalTodos > 0 ? <TriangleAlert size={18} /> : <CheckCircle2 size={18} />}
          <div>
            <h2>今日运维待办</h2>
            <span className="table-meta">汇总活动告警、证书预警、备份失败、近 72 小时 Runbook 异常与 14 天内到期 VPS</span>
          </div>
        </div>
        <div className="heading-actions">
          <span className={totalTodos > 0 ? "badge badge-bad" : "badge badge-ok"}>{totalTodos > 0 ? `${totalTodos} 项待处理` : "全部健康"}</span>
          <button className="icon-btn" type="button" title="刷新待办" aria-label="刷新待办" disabled={refreshingTodos} onClick={() => void loadActionCenter()}>
            <RefreshCw size={15} className={refreshingTodos ? "spin" : undefined} />
          </button>
        </div>
      </div>
      <div className="panel-body">
        {totalTodos === 0 ? (
          <div className="flex items-center justify-between gap-4">
            <p className="text-muted text-sm">没有活动告警、证书预警、失败备份或临期服务器。</p>
            {servers.length > 0 && actionCenter.enabledNotificationChannels === 0 && (
              <button className="btn" type="button" onClick={onSettings}><BellRing size={14} />配置手机告警渠道</button>
            )}
          </div>
        ) : (
          <div className="stack">
            {actionCenter.activeHostAlerts.slice(0, 5).map((alert) => (
              <div className="list-row" key={`host-${alert.id}`}>
                <div>
                  <span className="badge badge-bad mr-2">主机告警</span>
                  <strong>{alert.serverName}</strong>
                  <span className="table-meta">{alert.message} · {formatTime(alert.createdAt)}</span>
                </div>
                <div className="row-actions">
                  {onDiagnose && (
                    <button className="btn" type="button" onClick={() => onDiagnose(`请诊断 ${alert.serverName} 触发的主机告警原因，并给出安全的排查或修复命令。`, alert.serverId, alert.message)}>
                      <Sparkles size={14} />AI 诊断
                    </button>
                  )}
                  <button className="btn" type="button" onClick={onMonitoring}>查看监控</button>
                </div>
              </div>
            ))}
            {actionCenter.activeServiceEvents.slice(0, 5).map((event) => (
              <div className="list-row" key={`svc-${event.id}`}>
                <div>
                  <span className="badge badge-bad mr-2">服务异常</span>
                  <strong>{event.monitorName}</strong>
                  <span className="table-meta">{event.message} · {formatTime(event.createdAt)}</span>
                </div>
                <div className="row-actions">
                  {onDiagnose && (
                    <button className="btn" type="button" onClick={() => onDiagnose(`服务监控「${event.monitorName}」出现异常：${event.message}，请帮我排查原因。`, undefined, event.message)}>
                      <Sparkles size={14} />AI 诊断
                    </button>
                  )}
                  <button className="btn" type="button" onClick={onMonitoring}>查看探测</button>
                </div>
              </div>
            ))}
            {actionCenter.expiringCertificates.slice(0, 5).map((cert) => (
              <div className="list-row" key={`cert-${cert.monitorId}`}>
                <div>
                  <span className={cert.daysRemaining <= 7 ? "badge badge-bad mr-2" : "badge mr-2"}>证书预警</span>
                  <strong>{cert.monitorName}</strong>
                  <span className="table-meta">{cert.target} · {cert.daysRemaining < 0 ? `已过期 ${Math.abs(cert.daysRemaining)} 天` : `剩余 ${cert.daysRemaining} 天`}</span>
                </div>
                <button className="btn" type="button" onClick={onMonitoring}>查看证书</button>
              </div>
            ))}
            {actionCenter.failedBackups.slice(0, 5).map((item) => (
              <div className="list-row" key={`bak-${item.policyId}`}>
                <div>
                  <span className="badge badge-bad mr-2"><FolderArchive size={12} /> 备份失败</span>
                  <strong>{item.serverName} · {item.policyName}</strong>
                  <span className="table-meta">{item.error} · {formatTime(item.failedAt)}</span>
                </div>
                <button className="btn" type="button" onClick={onServers}>前往处理</button>
              </div>
            ))}
            {actionCenter.failedRunbooks.slice(0, 5).map((run) => (
              <div className="list-row" key={`rb-${run.executionId}`}>
                <div>
                  <span className="badge badge-bad mr-2"><ShieldAlert size={12} /> Runbook 异常</span>
                  <strong>{run.runbookName}</strong>
                  <span className="table-meta">{run.error || `${run.failureCount} 台目标失败`} · {formatTime(run.createdAt)}</span>
                </div>
                <button className="btn" type="button" onClick={onAutomation}>查看历史</button>
              </div>
            ))}
            {urgentExpiring.slice(0, 5).map((item) => (
              <div className="list-row" key={`exp-${item.serverId}`}>
                <div>
                  <span className={item.daysRemaining <= 7 ? "badge badge-bad mr-2" : "badge mr-2"}>续费提醒</span>
                  <strong>{item.serverName}</strong>
                  <span className="table-meta">{item.provider || "未填写服务商"} · {item.daysRemaining < 0 ? `已过期 ${Math.abs(item.daysRemaining)} 天` : item.daysRemaining === 0 ? "今天到期" : `${item.daysRemaining} 天后到期`}（{item.expiresAt}）</span>
                </div>
                <button className="btn" type="button" onClick={onServers}>管理资产</button>
              </div>
            ))}
            {actionCenter.enabledNotificationChannels === 0 && (
              <div className="list-row">
                <span className="table-meta">尚未启用 Telegram/飞书/钉钉/ntfy 等外部通知渠道，退出或离开电脑时无法收到手机推送。</span>
                <button className="btn" type="button" onClick={onSettings}><BellRing size={14} />配置通知中心</button>
              </div>
            )}
          </div>
        )}
      </div>
    </section>}

    {servers.length > 0 && <section className="panel cost-panel">
      <div className="panel-header"><h2>成本与到期</h2><span className="text-muted text-sm">依据服务器资料中的月成本与到期日</span></div>
      <div className="cost-body">
        <div className="cost-totals">
          {costEntries.size === 0 ? <p className="text-muted text-sm">还没有填写月度成本，编辑服务器即可补上。</p> : [...costEntries].map(([currency, entry]) => <div key={currency}>
            <span className="stat-label">{currency} 月度成本</span>
            <strong className="stat-value">{currencySymbol(currency)}{entry.total.toFixed(2)}</strong>
            <span className="stat-note">{entry.count} 台已登记{unpriced > 0 ? `，${unpriced} 台未填写` : ""}</span>
          </div>)}
        </div>
        <div className="cost-expiring">
          <h3>{expiring.length > 0 ? `${expiring.length} 台 30 天内到期` : "30 天内没有到期的服务器"}</h3>
          {expiring.length > 0 ? <ul>{expiring.slice(0, 5).map(({ server, days }) => <li key={server.id}>
            <button className="table-link" type="button" onClick={onServers}><span className="table-name">{server.name}</span><span className="table-meta">{server.provider || server.groupName || "未填写服务商"}</span></button>
            <span className={days < 0 || days <= 7 ? "badge badge-bad" : "badge"}>{days < 0 ? `已过期 ${Math.abs(days)} 天` : days === 0 ? "今天到期" : `${days} 天后`}</span>
          </li>)}</ul> : <p className="text-muted text-sm">填写到期日期后，这里会提前提醒续费。</p>}
        </div>
      </div>
    </section>}
    {loading ? <div className="skeleton" /> : servers.length === 0 ? <div className="empty"><div><ServerIcon size={34} /><h2>从第一台 VPS 开始</h2><p>支持 SSH 私钥、密码和系统 Agent 认证。</p><button className="btn btn-primary" type="button" onClick={onAdd}><Plus size={16} />添加 VPS</button></div></div> : <section>
      <div className="panel-header px-0"><h2>服务器清单</h2><button className="btn" type="button" onClick={onServers}>管理全部<ArrowRight size={15} /></button></div>
      <div className="table-wrap"><table className="data-table"><thead><tr><th>名称</th><th>地址</th><th>服务商</th><th>监控</th></tr></thead><tbody>{servers.slice(0, 6).map((server) => <tr key={server.id}><td><button className="table-link" type="button" onClick={onServers}><span className="table-name">{server.name}</span><span className="table-meta">{server.groupName || "未分组"}</span></button></td><td className="measured">{server.host}</td><td>{server.provider || "未填写"}</td><td><span className={metrics[server.id] ? "badge badge-ok" : "badge"}>{metrics[server.id] ? "已有读数" : "未检查"}</span></td></tr>)}</tbody></table></div>
    </section>}
    <div className="mt-8 flex items-start gap-3 text-muted text-sm"><ShieldCheck size={18} className="shrink-0" /><p>所有 SSH 连接都由本机发起。首次连接需确认主机密钥，应用不自动上传服务器清单。</p></div>
  </>;
}
