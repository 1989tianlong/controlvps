import { useCallback, useEffect, useState } from "react";
import { ArrowDownToLine, Boxes, Download, Globe, History, Play, Plus, RefreshCw, RotateCcw, Rocket, ScrollText, Search, Sparkles, Square, X } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import { buildComposeWithAutoHttps, composeAppTemplates, type ComposeAppTemplate } from "../data/composeTemplates";
import type { ComposeAction, ComposeLogs, ComposeOverview, ComposeProject, Server, TrustedExecutor } from "../types";
import { ComposeDeploymentEditor, ComposeDeploymentHistoryView } from "./ComposeDeployment";

export function ComposeManager({ server, trusted }: { server: Server; trusted: TrustedExecutor }) {
  const [overview, setOverview] = useState<ComposeOverview>();
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [logs, setLogs] = useState<ComposeLogs>();
  const [logQuery, setLogQuery] = useState("");
  const [editor, setEditor] = useState<{ project?: ComposeProject; preset?: { projectName: string; content: string } }>();
  const [historyProject, setHistoryProject] = useState<ComposeProject>();
  const [showTemplates, setShowTemplates] = useState(false);
  const [httpsDomain, setHttpsDomain] = useState("");

  const applyTemplate = (template: ComposeAppTemplate) => {
    const yaml = buildComposeWithAutoHttps(template, httpsDomain);
    setShowTemplates(false);
    setEditor({ preset: { projectName: template.id, content: yaml } });
  };

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
    setLogQuery("");
    try { setLogs(await trusted(server, (allow) => api.getComposeLogs(server.id, project.name, allow))); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };
  const downloadLogs = () => {
    if (!logs) return;
    const url = URL.createObjectURL(new Blob([logs.output], { type: "text/plain;charset=utf-8" }));
    const link = document.createElement("a");
    link.href = url;
    link.download = `${logs.project.replace(/[^a-zA-Z0-9_-]/g, "-")}-compose.log`;
    link.click();
    URL.revokeObjectURL(url);
  };
  const filteredLogOutput = logs?.output.split("\n").filter((line) => line.toLowerCase().includes(logQuery.toLowerCase())).join("\n") ?? "";

  return <div className="compose-manager">
    <div className="panel-header"><div><h2>Docker Compose 应用</h2><span className="table-meta">{overview ? `Compose ${overview.version} · ${overview.projects.length} 个项目` : "正在通过 SSH 检测 Compose v2"}</span></div><div className="heading-actions"><button className="btn" type="button" disabled={busy !== ""} onClick={() => void load()}><RefreshCw size={15} />刷新</button><button className="btn" type="button" disabled={busy !== ""} onClick={() => setShowTemplates((v) => !v)}><Sparkles size={15} />精选模板 / 自动 HTTPS</button><button className="btn btn-primary" type="button" disabled={busy !== ""} onClick={() => setEditor({})}><Plus size={15} />新建应用</button></div></div>
    {showTemplates && (
      <section className="panel panel-body m-4">
        <div className="flex flex-wrap items-center justify-between gap-3 mb-3">
          <div>
            <h3>精选自托管应用模板（8 套）</h3>
            <p className="table-meta">填写可选域名后，自动附加 Caddy 2 反向代理与 Let&apos;s Encrypt 自动 HTTPS 证书。</p>
          </div>
          <div className="row-actions">
            <label className="search-field" style={{ minWidth: 240 }}>
              <Globe size={14} aria-hidden="true" />
              <input className="input" value={httpsDomain} onChange={(e) => setHttpsDomain(e.target.value)} placeholder="可选域名（如 status.example.com）" />
            </label>
            <button className="icon-btn" type="button" aria-label="关闭模板面板" onClick={() => setShowTemplates(false)}><X size={16} /></button>
          </div>
        </div>
        <div className="docker-service-grid">
          {composeAppTemplates.map((tpl) => (
            <article className="docker-service" key={tpl.id}>
              <div>
                <strong>{tpl.name}</strong> <span className="badge">{tpl.category}</span>
                <p className="table-meta mt-1">{tpl.summary}</p>
              </div>
              <button className="btn btn-primary" type="button" onClick={() => applyTemplate(tpl)}>
                <Rocket size={14} />
                {httpsDomain.trim() ? "带自动 HTTPS 部署" : "使用模板"}
              </button>
            </article>
          ))}
        </div>
      </section>
    )}
    {error && <div className="error-banner m-4" role="alert"><p>{error}</p><button className="btn" type="button" onClick={() => void load()}>重试</button></div>}
    {notice && <div className="file-notice docker-notice" role="status">{notice}</div>}
    {!overview && !error && <div className="skeleton docker-skeleton" aria-label="正在读取 Compose 项目" />}
    {overview && overview.projects.length === 0 && <div className="empty"><div><Boxes size={30} /><h2>没有发现 Compose 项目</h2><p>可从本地 YAML 或精选模板新建应用，也可以先在服务器使用 docker compose up -d 部署后刷新。</p><div className="row-actions"><button className="btn" type="button" onClick={() => setShowTemplates(true)}><Sparkles size={16} />浏览精选模板</button><button className="btn btn-primary" type="button" onClick={() => setEditor({})}><Plus size={16} />新建 Compose 应用</button></div></div></div>}
    {overview && overview.projects.length > 0 && <div className="table-wrap"><table className="data-table compose-table"><thead><tr><th>项目</th><th>状态</th><th>配置文件</th><th className="text-right">发布与操作</th></tr></thead><tbody>{overview.projects.map((project) => <tr key={project.name}><td><span className="table-name">{project.name}</span><span className="table-meta">{project.workingDirectory}</span></td><td><span className={project.status.toLowerCase().includes("running") ? "badge badge-ok" : "badge"}>{project.status}</span></td><td>{project.configFiles.map((path) => <code className="compose-path" key={path}>{path}</code>)}</td><td><div className="row-actions"><button className="btn compose-deploy-button" type="button" title="校验配置并发布" disabled={busy !== ""} onClick={() => setEditor({ project })}><Rocket size={15} />发布</button><button className="icon-btn" type="button" title="部署历史与回滚" aria-label={`查看 ${project.name} 部署历史`} disabled={busy !== ""} onClick={() => setHistoryProject(project)}><History size={16} /></button><button className="icon-btn" type="button" title="查看项目日志" aria-label={`查看 ${project.name} 项目日志`} disabled={busy !== ""} onClick={() => void showLogs(project)}><ScrollText size={16} /></button><button className="icon-btn" type="button" title="拉取镜像" aria-label={`拉取 ${project.name} 镜像`} disabled={busy !== ""} onClick={() => void action(project, "pull")}><ArrowDownToLine size={16} /></button><button className="icon-btn" type="button" title="启动或按配置更新" aria-label={`启动或更新 ${project.name}`} disabled={busy !== ""} onClick={() => void action(project, "up")}><Play size={16} /></button><button className="icon-btn" type="button" title="重启项目" aria-label={`重启 ${project.name} 项目`} disabled={busy !== ""} onClick={() => void action(project, "restart")}><RotateCcw size={16} /></button><button className="icon-btn" type="button" title="停止并移除项目容器" aria-label={`停止 ${project.name} 项目`} disabled={busy !== ""} onClick={() => void action(project, "down")}><Square size={15} /></button></div></td></tr>)}</tbody></table></div>}
    {overview && overview.projects.length > 0 && <p className="compose-note">“发布”支持 YAML 校验、服务变更预检、配置快照与历史回滚；“停止”会移除项目容器和网络，但不删除命名卷。快捷操作沿用服务器现有配置。</p>}
    {logs && <section className="docker-logs"><div className="panel-header"><div><h3>{logs.project} · 最近 200 行项目日志</h3><span className="table-meta">已按项目环境变量自动脱敏</span></div><div className="row-actions"><button className="icon-btn" type="button" title="下载日志" aria-label="下载 Compose 日志" onClick={downloadLogs}><Download size={16} /></button><button className="icon-btn" type="button" aria-label="关闭项目日志" onClick={() => setLogs(undefined)}><X size={17} /></button></div></div><div className="compose-log-filter"><div className="search-wrap"><Search size={15} /><input className="input" type="search" aria-label="搜索 Compose 日志" placeholder="筛选当前日志" value={logQuery} onChange={(event) => setLogQuery(event.target.value)} /></div><span className="table-meta">{logQuery ? `${filteredLogOutput.split("\n").filter(Boolean).length} 行匹配` : "全部日志"}</span></div><pre className="action-output docker-log-output"><code>{filteredLogOutput || (logQuery ? "没有匹配日志" : "项目没有日志输出")}</code></pre></section>}
    {editor && <ComposeDeploymentEditor server={server} project={editor.project} preset={editor.preset} trusted={trusted} onClose={() => setEditor(undefined)} onChanged={() => { void load(false); }} />}
    {historyProject && <ComposeDeploymentHistoryView server={server} project={historyProject} trusted={trusted} onClose={() => setHistoryProject(undefined)} onChanged={() => { void load(false); }} />}
  </div>;
}
