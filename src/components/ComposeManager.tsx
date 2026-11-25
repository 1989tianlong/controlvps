import { useCallback, useEffect, useState } from "react";
import { ArrowDownToLine, Boxes, Play, RefreshCw, RotateCcw, ScrollText, Square, X } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import type { ComposeAction, ComposeLogs, ComposeOverview, ComposeProject, Server, TrustedExecutor } from "../types";

export function ComposeManager({ server, trusted }: { server: Server; trusted: TrustedExecutor }) {
  const [overview, setOverview] = useState<ComposeOverview>();
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [logs, setLogs] = useState<ComposeLogs>();

  const load = useCallback(async (showBusy = true) => {
    if (showBusy) setBusy("list");
    setError("");
    try { setOverview(await trusted(server, (allow) => api.listComposeProjects(server.id, allow))); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { if (showBusy) setBusy(""); }
  }, [server, trusted]);
  useEffect(() => { void load(); }, [load]);

  const action = async (project: ComposeProject, action: ComposeAction) => {
    if (action === "down" && !window.confirm(`确认停止并移除 ${project.name} 的 Compose 容器和网络？不会删除命名卷或镜像。`)) return;
    if (action === "restart" && !window.confirm(`确认重启 Compose 项目 ${project.name}？运行中的服务会中断。`)) return;
    setBusy(`${project.name}-${action}`);
    setError("");
    setNotice("");
    try {
      await trusted(server, (allow) => api.runComposeAction(server.id, project.name, action, allow));
      const label = action === "up" ? "启动或更新" : action === "down" ? "停止" : action === "restart" ? "重启" : "拉取镜像";
      setNotice(`项目 ${project.name} 已完成：${label}`);
      await load(false);
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };
  const showLogs = async (project: ComposeProject) => {
    setBusy(`${project.name}-logs`);
    setError("");
    try { setLogs(await trusted(server, (allow) => api.getComposeLogs(server.id, project.name, allow))); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };

  return <div className="compose-manager">
    <div className="panel-header"><div><h2>Docker Compose 项目</h2><span className="table-meta">{overview ? `Compose ${overview.version} · ${overview.projects.length} 个项目` : "正在通过 SSH 检测 Compose v2"}</span></div><button className="btn" type="button" disabled={busy !== ""} onClick={() => void load()}><RefreshCw size={15} />刷新</button></div>
    {error && <div className="error-banner m-4" role="alert"><p>{error}</p><button className="btn" type="button" onClick={() => void load()}>重试</button></div>}
    {notice && <div className="file-notice docker-notice" role="status">{notice}</div>}
    {!overview && !error && <div className="skeleton docker-skeleton" aria-label="正在读取 Compose 项目" />}
    {overview && overview.projects.length === 0 && <div className="empty"><div><Boxes size={30} /><h2>没有发现 Compose 项目</h2><p>Compose v2 会识别由它创建且仍保留配置路径的项目。可先在服务器上使用 docker compose up -d 部署。</p></div></div>}
    {overview && overview.projects.length > 0 && <div className="table-wrap"><table className="data-table compose-table"><thead><tr><th>项目</th><th>状态</th><th>配置文件</th><th className="text-right">操作</th></tr></thead><tbody>{overview.projects.map((project) => <tr key={project.name}><td><span className="table-name">{project.name}</span></td><td><span className={project.status.toLowerCase().includes("running") ? "badge badge-ok" : "badge"}>{project.status}</span></td><td>{project.configFiles.map((path) => <code className="compose-path" key={path}>{path}</code>)}</td><td><div className="row-actions"><button className="icon-btn" type="button" title="查看项目日志" aria-label={`查看 ${project.name} 项目日志`} disabled={busy !== ""} onClick={() => void showLogs(project)}><ScrollText size={16} /></button><button className="icon-btn" type="button" title="拉取镜像" aria-label={`拉取 ${project.name} 镜像`} disabled={busy !== ""} onClick={() => void action(project, "pull")}><ArrowDownToLine size={16} /></button><button className="icon-btn" type="button" title="启动或按配置更新" aria-label={`启动或更新 ${project.name}`} disabled={busy !== ""} onClick={() => void action(project, "up")}><Play size={16} /></button><button className="icon-btn" type="button" title="重启项目" aria-label={`重启 ${project.name} 项目`} disabled={busy !== ""} onClick={() => void action(project, "restart")}><RotateCcw size={16} /></button><button className="icon-btn" type="button" title="停止并移除项目容器" aria-label={`停止 ${project.name} 项目`} disabled={busy !== ""} onClick={() => void action(project, "down")}><Square size={15} /></button></div></td></tr>)}</tbody></table></div>}
    {overview && overview.projects.length > 0 && <p className="compose-note">“启动或更新”执行 up -d；“停止”执行 down，会移除项目容器和网络，但不删除命名卷。所有操作只使用服务器上已有的 Compose 配置文件。</p>}
    {logs && <section className="docker-logs"><div className="panel-header"><div><h3>{logs.project} · 最近 200 行项目日志</h3></div><button className="icon-btn" type="button" aria-label="关闭项目日志" onClick={() => setLogs(undefined)}><X size={17} /></button></div><pre className="action-output docker-log-output"><code>{logs.output || "项目没有日志输出"}</code></pre></section>}
  </div>;
}
