import { ArrowRight, Compass, Plus, Server as ServerIcon, ShieldCheck } from "lucide-react";
import type { Server, ServerMetrics } from "../types";

interface Props {
  servers: Server[];
  loading: boolean;
  metrics: Record<string, ServerMetrics>;
  onAdd: () => void;
  onServers: () => void;
  onDiscover: () => void;
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

export function OverviewPage({ servers, loading, metrics, onAdd, onServers, onDiscover }: Props) {
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
  return <>
    <div className="page-heading"><div><h1>VPS 工作台</h1><p>从同一个本地工作区管理连接与运行状态。</p></div><div className="heading-actions"><button className="btn" type="button" onClick={onDiscover}><Compass size={16} />发现 VPS</button><button className="btn btn-primary" type="button" onClick={onAdd}><Plus size={16} />添加 VPS</button></div></div>
    <div className="stats">
      <div className="stat"><span className="stat-label">已保存服务器</span><strong className="stat-value">{servers.length}</strong><span className="stat-note">本机 SQLite</span></div>
      <div className="stat"><span className="stat-label">资产分组</span><strong className="stat-value">{groups}</strong><span className="stat-note">按用途组织</span></div>
      <div className="stat"><span className="stat-label">已有监控读数</span><strong className="stat-value">{measured}</strong><span className="stat-note">本次应用运行</span></div>
      <div className="stat"><span className="stat-label">凭据保护</span><strong className="stat-value text-base">系统钥匙串</strong><span className="stat-note">不进入普通配置</span></div>
    </div>
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
