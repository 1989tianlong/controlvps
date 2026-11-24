import { useCallback, useEffect, useState } from "react";
import { Box, Play, RefreshCw, RotateCcw, ScrollText, Search, Square, X } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import type { DockerAction, DockerContainer, DockerLogs, DockerOverview, Server, TrustedExecutor } from "../types";

interface Props {
  server: Server;
  trusted: TrustedExecutor;
}

export function DockerManager({ server, trusted }: Props) {
  const [overview, setOverview] = useState<DockerOverview>();
  const [query, setQuery] = useState("");
  const [stateFilter, setStateFilter] = useState("all");
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [logs, setLogs] = useState<DockerLogs>();
  const [logsName, setLogsName] = useState("");

  const load = useCallback(async (showBusy = true) => {
    if (showBusy) setBusy("list");
    setError("");
    try {
      setOverview(await trusted(server, (allow) => api.listDockerContainers(server.id, allow)));
    } catch (reason) { setError(errorMessage(reason)); }
    finally { if (showBusy) setBusy(""); }
  }, [server, trusted]);
  useEffect(() => { void load(); }, [load]);

  const action = async (container: DockerContainer, action: DockerAction) => {
    if (action !== "start" && !window.confirm(`确认${action === "stop" ? "停止" : "重启"}容器 ${container.name}？运行中的服务可能中断。`)) return;
    setBusy(container.id);
    setError("");
    setNotice("");
    try {
      await trusted(server, (allow) => api.runDockerAction(server.id, container.id, action, allow));
      setNotice(`容器 ${container.name} 已执行${action === "start" ? "启动" : action === "stop" ? "停止" : "重启"}`);
      await load(false);
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };
  const showLogs = async (container: DockerContainer) => {
    setBusy(container.id);
    setError("");
    try {
      const result = await trusted(server, (allow) => api.getDockerLogs(server.id, container.id, allow));
      setLogs(result);
      setLogsName(container.name);
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };

  const containers = overview?.containers.filter((container) => {
    const matchesQuery = `${container.name} ${container.image}`.toLowerCase().includes(query.toLowerCase());
    const matchesState = stateFilter === "all" || (stateFilter === "running" ? container.state === "running" : container.state !== "running");
    return matchesQuery && matchesState;
  }) ?? [];
  return <div className="docker-manager">
    <div className="panel-header"><div><h2>Docker 容器</h2><span className="table-meta">{overview ? `Docker ${overview.version} · ${overview.containers.length} 个容器` : "正在通过 SSH 检测 Docker"}</span></div><button className="btn" type="button" disabled={busy !== ""} onClick={() => void load()}><RefreshCw size={15} />刷新</button></div>
    {error && <div className="error-banner m-4" role="alert"><p>{error}</p><button className="btn" type="button" onClick={() => void load()}>重试</button></div>}
    {notice && <div className="file-notice docker-notice" role="status">{notice}</div>}
    {overview && <div className="docker-filters"><div className="search-wrap"><Search size={16} /><input className="input" type="search" aria-label="搜索 Docker 容器" placeholder="搜索名称或镜像" value={query} onChange={(event) => setQuery(event.target.value)} /></div><select className="select" aria-label="按容器状态筛选" value={stateFilter} onChange={(event) => setStateFilter(event.target.value)}><option value="all">全部状态</option><option value="running">仅运行中</option><option value="stopped">仅已停止</option></select></div>}
    {!overview && !error && <div className="skeleton docker-skeleton" aria-label="正在读取 Docker 容器" />}
    {overview && overview.containers.length === 0 && <div className="empty"><div><Box size={30} /><h2>没有 Docker 容器</h2><p>服务器已安装 Docker，但当前没有运行或停止的容器。</p></div></div>}
    {containers.length > 0 && <div className="table-wrap"><table className="data-table docker-table"><thead><tr><th>容器</th><th>状态</th><th>CPU / 内存</th><th>PIDs</th><th className="text-right">操作</th></tr></thead><tbody>{containers.map((container) => <tr key={container.id}><td><span className="table-name">{container.name}</span><span className="table-meta" title={container.image}>{container.image}</span>{container.ports && <span className="table-meta" title={container.ports}>{container.ports}</span>}</td><td><span className={container.state === "running" ? "badge badge-ok" : "badge"}>{container.state === "running" ? "运行中" : "已停止"}</span><span className="table-meta" title={container.status}>{container.status}</span></td><td className="measured">{container.cpuPercent === undefined ? "—" : `${container.cpuPercent.toFixed(1)}% / ${container.memoryPercent?.toFixed(1) ?? "—"}%`}{container.memoryUsage && <span className="table-meta">{container.memoryUsage}</span>}</td><td className="measured">{container.pids ?? "—"}</td><td><div className="row-actions"><button className="icon-btn" type="button" title="查看日志" aria-label={`查看 ${container.name} 日志`} disabled={busy !== ""} onClick={() => void showLogs(container)}><ScrollText size={16} /></button>{container.state === "running" ? <><button className="icon-btn" type="button" title="重启容器" aria-label={`重启 ${container.name}`} disabled={busy !== ""} onClick={() => void action(container, "restart")}><RotateCcw size={16} /></button><button className="icon-btn" type="button" title="停止容器" aria-label={`停止 ${container.name}`} disabled={busy !== ""} onClick={() => void action(container, "stop")}><Square size={15} /></button></> : <button className="icon-btn" type="button" title="启动容器" aria-label={`启动 ${container.name}`} disabled={busy !== ""} onClick={() => void action(container, "start")}><Play size={16} /></button>}</div></td></tr>)}</tbody></table></div>}
    {overview && containers.length === 0 && overview.containers.length > 0 && <div className="docker-no-match"><Search size={22} /><p>没有匹配的容器</p></div>}
    {logs && <section className="docker-logs"><div className="panel-header"><div><h3>{logsName} · 最近 200 行日志</h3></div><button className="icon-btn" type="button" aria-label="关闭日志" onClick={() => setLogs(undefined)}><X size={17} /></button></div><pre className="action-output docker-log-output"><code>{logs.output || "容器没有日志输出"}</code></pre></section>}
  </div>;
}
