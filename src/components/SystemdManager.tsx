import { useCallback, useEffect, useState } from "react";
import { Play, RefreshCw, RotateCcw, ScrollText, Search, Settings2, Square, X } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import type { Server, SystemdAction, SystemdLogs, SystemdOverview, SystemdService, TrustedExecutor } from "../types";

interface Props {
  server: Server;
  trusted: TrustedExecutor;
}

export function SystemdManager({ server, trusted }: Props) {
  const [overview, setOverview] = useState<SystemdOverview>();
  const [query, setQuery] = useState("");
  const [stateFilter, setStateFilter] = useState("all");
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [logs, setLogs] = useState<SystemdLogs>();

  const load = useCallback(async (showBusy = true) => {
    if (showBusy) setBusy("list");
    setError("");
    try {
      setOverview(await trusted(server, (allow) => api.listSystemdServices(server.id, allow)));
    } catch (reason) { setError(errorMessage(reason)); }
    finally { if (showBusy) setBusy(""); }
  }, [server, trusted]);
  useEffect(() => {
    setOverview(undefined);
    setLogs(undefined);
    void load();
  }, [load]);

  const action = async (service: SystemdService, action: SystemdAction) => {
    if (action !== "start" && !window.confirm(`确认${action === "stop" ? "停止" : "重启"}服务 ${service.name}？相关业务可能中断。`)) return;
    setBusy(service.name);
    setError("");
    setNotice("");
    try {
      await trusted(server, (allow) => api.runSystemdAction(server.id, service.name, action, allow));
      setNotice(`服务 ${service.name} 已执行${action === "start" ? "启动" : action === "stop" ? "停止" : "重启"}`);
      await load(false);
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };
  const showLogs = async (service: SystemdService) => {
    setBusy(service.name);
    setError("");
    try {
      setLogs(await trusted(server, (allow) => api.getSystemdLogs(server.id, service.name, allow)));
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };

  const services = overview?.services.filter((service) => {
    const matchesQuery = `${service.name} ${service.description}`.toLowerCase().includes(query.toLowerCase());
    const matchesState = stateFilter === "all" || (stateFilter === "inactive" ? service.active !== "active" : service.active === stateFilter);
    return matchesQuery && matchesState;
  }) ?? [];
  return <div className="docker-manager">
    <div className="panel-header"><div><h2>systemd 服务</h2><span className="table-meta">{overview ? `${overview.version} · ${overview.services.length} 个服务` : "正在通过 SSH 检测 systemd"}</span></div><button className="btn" type="button" disabled={busy !== ""} onClick={() => void load()}><RefreshCw size={15} />刷新</button></div>
    {error && <div className="error-banner m-4" role="alert"><p>{error}</p><button className="btn" type="button" onClick={() => void load()}>重试</button></div>}
    {notice && <div className="file-notice docker-notice" role="status">{notice}</div>}
    {overview && <div className="docker-filters"><div className="search-wrap"><Search size={16} /><input className="input" type="search" aria-label="搜索 systemd 服务" placeholder="搜索服务名称或描述" value={query} onChange={(event) => setQuery(event.target.value)} /></div><select className="select" aria-label="按服务状态筛选" value={stateFilter} onChange={(event) => setStateFilter(event.target.value)}><option value="all">全部状态</option><option value="active">仅运行中</option><option value="failed">仅失败</option><option value="inactive">仅非运行中</option></select></div>}
    {!overview && !error && <div className="skeleton docker-skeleton" aria-label="正在读取 systemd 服务" />}
    {overview && overview.services.length === 0 && <div className="empty"><div><Settings2 size={30} /><h2>没有 systemd 服务</h2><p>服务器没有返回已加载的服务单元。</p></div></div>}
    {services.length > 0 && <div className="table-wrap"><table className="data-table docker-table"><thead><tr><th>服务</th><th>状态</th><th>加载 / 开机启动</th><th className="text-right">操作</th></tr></thead><tbody>{services.map((service) => <tr key={service.name}><td><span className="table-name measured">{service.name}</span><span className="table-meta" title={service.description}>{service.description || "无描述"}</span></td><td><span className={service.active === "active" ? "badge badge-ok" : service.active === "failed" ? "badge badge-bad" : "badge"}>{service.active === "active" ? "运行中" : service.active === "failed" ? "失败" : service.active}</span><span className="table-meta">{service.sub}</span></td><td className="measured">{service.load}<span className="table-meta">{service.enabled}</span></td><td><div className="row-actions"><button className="icon-btn" type="button" title="查看日志" aria-label={`查看 ${service.name} 日志`} disabled={busy !== ""} onClick={() => void showLogs(service)}><ScrollText size={16} /></button>{service.active === "active" ? <><button className="icon-btn" type="button" title="重启服务" aria-label={`重启 ${service.name}`} disabled={busy !== ""} onClick={() => void action(service, "restart")}><RotateCcw size={16} /></button><button className="icon-btn" type="button" title="停止服务" aria-label={`停止 ${service.name}`} disabled={busy !== ""} onClick={() => void action(service, "stop")}><Square size={15} /></button></> : <><button className="icon-btn" type="button" title="启动服务" aria-label={`启动 ${service.name}`} disabled={busy !== ""} onClick={() => void action(service, "start")}><Play size={16} /></button><button className="icon-btn" type="button" title="重启服务" aria-label={`重启 ${service.name}`} disabled={busy !== ""} onClick={() => void action(service, "restart")}><RotateCcw size={16} /></button></>}</div></td></tr>)}</tbody></table></div>}
    {overview && services.length === 0 && overview.services.length > 0 && <div className="docker-no-match"><Search size={22} /><p>没有匹配的服务</p></div>}
    {logs && <section className="docker-logs"><div className="panel-header"><div><h3>{logs.unit} · 最近 200 行日志</h3></div><button className="icon-btn" type="button" aria-label="关闭日志" onClick={() => setLogs(undefined)}><X size={17} /></button></div><pre className="action-output docker-log-output"><code>{logs.output || "服务没有日志输出"}</code></pre></section>}
  </div>;
}
