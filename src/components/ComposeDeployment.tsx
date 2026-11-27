import { useEffect, useMemo, useRef, useState } from "react";
import { AlertTriangle, ArrowDownToLine, CheckCircle2, History, KeyRound, LoaderCircle, Plus, Rocket, RotateCcw, ShieldCheck, Trash2, X } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import { formatTime } from "../lib/format";
import type { ComposeApplication, ComposeDeploymentHistory, ComposeDeploymentPlan, ComposeDeploymentResult, ComposeEditInput, ComposeEnvironmentEntry, ComposeProject, Server, TrustedExecutor } from "../types";

const defaultYaml = `services:
  web:
    image: nginx:1.27-alpine
    restart: unless-stopped
    ports:
      - "127.0.0.1:8080:80"
`;

interface EditorProps {
  server: Server;
  project?: ComposeProject;
  preset?: { projectName: string; content: string };
  trusted: TrustedExecutor;
  onClose: () => void;
  onChanged: () => void;
}

export function ComposeDeploymentEditor({ server, project, preset, trusted, onClose, onChanged }: EditorProps) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [loading, setLoading] = useState(Boolean(project));
  const [created, setCreated] = useState(false);
  const [projectName, setProjectName] = useState(project?.name ?? preset?.projectName ?? "");
  const [workingDirectory, setWorkingDirectory] = useState(
    project?.workingDirectory ?? (preset?.projectName ? `~/controlvps-apps/${preset.projectName}` : "~/controlvps-apps/app"),
  );
  const [directoryTouched, setDirectoryTouched] = useState(Boolean(project));
  const [configFiles, setConfigFiles] = useState<string[]>(project?.configFiles.filter((path) => !path.includes("/.controlvps/rollback-")) ?? []);
  const [content, setContent] = useState(preset?.content ?? defaultYaml);
  const [revision, setRevision] = useState("");
  const [environment, setEnvironment] = useState<ComposeEnvironmentEntry[]>([]);
  const [changes, setChanges] = useState<Record<string, string | null>>({});
  const [newKey, setNewKey] = useState("");
  const [newValue, setNewValue] = useState("");
  const [plan, setPlan] = useState<ComposeDeploymentPlan>();
  const [reviewedSignature, setReviewedSignature] = useState("");
  const [result, setResult] = useState<ComposeDeploymentResult>();
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");

  useEffect(() => {
    dialogRef.current?.showModal();
  }, []);

  useEffect(() => {
    if (!project) return;
    let disposed = false;
    setLoading(true);
    setError("");
    void trusted(server, (allow) => api.getComposeApplication(server.id, project.name, allow))
      .then((application: ComposeApplication) => {
        if (disposed) return;
        setProjectName(application.project.name);
        setWorkingDirectory(application.project.workingDirectory);
        setConfigFiles(application.project.configFiles.filter((path) => !path.includes("/.controlvps/rollback-")));
        setContent(application.content);
        setRevision(application.revision);
        setEnvironment(application.environment);
      })
      .catch((reason) => { if (!disposed) setError(errorMessage(reason)); })
      .finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; };
  }, [project, server, trusted]);

  const input = useMemo<ComposeEditInput>(() => ({
    projectName: projectName.trim(),
    workingDirectory: workingDirectory.trim(),
    configFiles,
    content,
    environmentChanges: Object.entries(changes).map(([key, value]) => ({ key, value })),
    expectedRevision: revision,
  }), [changes, configFiles, content, projectName, revision, workingDirectory]);
  const signature = JSON.stringify(input);

  const invalidate = () => {
    setPlan(undefined);
    setReviewedSignature("");
    setResult(undefined);
    setNotice("");
  };
  const updateProjectName = (value: string) => {
    invalidate();
    setProjectName(value);
    if (!directoryTouched) {
      const safeName = value.trim().replace(/[^a-zA-Z0-9_-]/g, "-") || "app";
      setWorkingDirectory(`~/controlvps-apps/${safeName}`);
    }
  };
  const setEnvironmentValue = (key: string, value: string) => {
    invalidate();
    setChanges((current) => ({ ...current, [key]: value }));
  };
  const removeEnvironment = (key: string) => {
    invalidate();
    setChanges((current) => {
      const next = { ...current };
      if (next[key] === null) delete next[key];
      else next[key] = null;
      return next;
    });
  };
  const addEnvironment = () => {
    const key = newKey.trim();
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) {
      setError("环境变量名需以字母或下划线开头，只能包含字母、数字和下划线");
      return;
    }
    if (environment.some((entry) => entry.key === key)) {
      setError(`环境变量 ${key} 已存在，请直接修改`);
      return;
    }
    invalidate();
    setError("");
    setEnvironment((current) => [...current, { key, managed: true }].sort((left, right) => left.key.localeCompare(right.key)));
    setChanges((current) => ({ ...current, [key]: newValue }));
    setNewKey("");
    setNewValue("");
  };
  const importYaml = async (file?: File) => {
    if (!file) return;
    if (file.size > 1024 * 1024) {
      setError("Compose YAML 不能超过 1 MiB");
      return;
    }
    try {
      const text = await file.text();
      invalidate();
      setContent(text);
      setError("");
    } catch (reason) { setError(errorMessage(reason)); }
  };
  const preview = async () => {
    setBusy("preview");
    setError("");
    setNotice("");
    setResult(undefined);
    try {
      const nextPlan = await trusted(server, (allow) => api.previewComposeDeployment(server.id, input, allow));
      setPlan(nextPlan);
      setReviewedSignature(signature);
    } catch (reason) {
      setPlan(undefined);
      setReviewedSignature("");
      setError(errorMessage(reason));
    } finally { setBusy(""); }
  };
  const deploy = async () => {
    if (!plan || reviewedSignature !== signature) return;
    const confirmed = window.confirm(`确认在 ${server.name} 部署 ${projectName}？\n\n新建 ${plan.createCount} 个，更新 ${plan.updateCount} 个，移除 ${plan.removeCount} 个服务。\n将先保存当前配置快照；不会删除命名卷，也不会回滚数据库或持久化数据。`);
    if (!confirmed) return;
    setBusy("deploy");
    setError("");
    setNotice("");
    try {
      const deployment = await trusted(server, (allow) => api.deployComposeApplication(server.id, input, allow));
      setResult(deployment);
      setCreated(true);
      setRevision(deployment.record.revisionAfter ?? revision);
      setPlan(undefined);
      setReviewedSignature("");
      setChanges({});
      setEnvironment((current) => current.filter((entry) => changes[entry.key] !== null).map((entry) => ({ ...entry, managed: entry.managed || Object.prototype.hasOwnProperty.call(changes, entry.key) })));
      if (deployment.record.status === "succeeded") setNotice(`部署完成，历史 ID：${deployment.record.id}`);
      else setError(`部署命令失败，历史 ID：${deployment.record.id}。配置已写入，可检查输出或到部署历史回滚。`);
      onChanged();
    } catch (reason) { setError(`${errorMessage(reason)}。如命令已开始执行，请到部署历史核对实际状态。`); }
    finally { setBusy(""); }
  };

  const locked = Boolean(project) || created;
  return <dialog ref={dialogRef} className="dialog compose-deployment-dialog" onCancel={(event) => { event.preventDefault(); if (!busy) onClose(); }}>
    <div className="dialog-header"><div><h2>{locked ? `发布 ${projectName}` : "新建 Compose 应用"}</h2><span className="table-meta">{server.name} · 配置预检、快照与部署</span></div><button className="icon-btn" type="button" aria-label="关闭发布编辑器" disabled={Boolean(busy)} onClick={onClose}><X size={18} /></button></div>
    <div className="dialog-body">
      {error && <div className="error-banner" role="alert">{error}</div>}
      {notice && <div className="file-notice" role="status">{notice}</div>}
      {loading ? <div className="skeleton compose-editor-skeleton" aria-label="正在读取 Compose 配置" /> : <>
        <div className="form-grid">
          <div className="field"><label htmlFor="compose-project-name">项目名称</label><input id="compose-project-name" className="input" value={projectName} disabled={locked || Boolean(busy)} onChange={(event) => updateProjectName(event.target.value)} placeholder="例如 blog" /></div>
          <div className="field"><label htmlFor="compose-working-directory">远程项目目录</label><input id="compose-working-directory" className="input" value={workingDirectory} disabled={locked || Boolean(busy)} onChange={(event) => { invalidate(); setDirectoryTouched(true); setWorkingDirectory(event.target.value); }} placeholder="~/controlvps-apps/blog" /><p className="field-help">支持绝对路径或 ~/，只创建该项目目录。</p></div>
        </div>
        <section className="form-section">
          <div className="section-heading"><div><h2>Compose YAML</h2><span className="table-meta">{configFiles.length > 1 ? `编辑主配置，其余 ${configFiles.length - 1} 个文件保持不变` : "服务端使用 docker compose config 校验"}</span></div><label className={`btn compose-file-picker${busy ? " disabled" : ""}`}><ArrowDownToLine size={15} />导入本地 YAML<input type="file" accept=".yaml,.yml,text/yaml" disabled={Boolean(busy)} onChange={(event) => { void importYaml(event.target.files?.[0]); event.target.value = ""; }} /></label></div>
          <textarea className="textarea compose-yaml-editor" aria-label="Compose YAML" value={content} disabled={Boolean(busy)} onChange={(event) => { invalidate(); setContent(event.target.value); }} spellCheck={false} />
        </section>
        <section className="form-section">
          <div className="section-heading"><div><h2>环境变量</h2><span className="table-meta">值不会回显；编辑后写入远程 .env 和 macOS Keychain</span></div><KeyRound size={20} /></div>
          {environment.length === 0 && <p className="compose-muted">没有声明环境变量。可添加变量，或保持项目不使用 .env。</p>}
          <div className="compose-environment-list">{environment.map((entry) => {
            const removed = changes[entry.key] === null;
            const changed = Object.prototype.hasOwnProperty.call(changes, entry.key) && !removed;
            return <div className="compose-environment-row" data-removed={removed} key={entry.key}><code>{entry.key}</code><input className="input" type="password" aria-label={`更新 ${entry.key}`} value={removed ? "" : changes[entry.key] ?? ""} disabled={Boolean(busy) || removed} placeholder={changed ? "已设置新值" : entry.managed ? "Keychain 已保护；输入以替换" : "远程值保留；输入以替换"} onChange={(event) => setEnvironmentValue(entry.key, event.target.value)} autoComplete="off" /><span className={entry.managed || changed ? "badge badge-ok" : "badge"}>{removed ? "将删除" : changed ? "将更新" : entry.managed ? "已保护" : "仅隐藏"}</span><button className="icon-btn" type="button" title={removed ? "保留此变量" : "删除此变量"} aria-label={`${removed ? "恢复" : "删除"} ${entry.key}`} disabled={Boolean(busy)} onClick={() => removeEnvironment(entry.key)}>{removed ? <RotateCcw size={15} /> : <Trash2 size={15} />}</button></div>;
          })}</div>
          <div className="compose-environment-add"><input className="input" aria-label="新环境变量名" placeholder="变量名" value={newKey} disabled={Boolean(busy)} onChange={(event) => setNewKey(event.target.value)} autoComplete="off" /><input className="input" type="password" aria-label="新环境变量值" placeholder="变量值" value={newValue} disabled={Boolean(busy)} onChange={(event) => setNewValue(event.target.value)} autoComplete="off" /><button className="btn" type="button" disabled={Boolean(busy) || !newKey.trim()} onClick={addEnvironment}><Plus size={15} />添加</button></div>
          <p className="field-help">修改变量会规范化整个 .env 文件；包含复杂写法的行需先在服务器整理。任何值都不会写入操作记录。</p>
        </section>
        {plan && <ComposePlanView plan={plan} stale={reviewedSignature !== signature} />}
        {result && <section className="compose-deployment-result"><div className="section-heading"><h2>{result.record.status === "succeeded" ? "部署输出" : "失败输出"}</h2><span className={result.record.status === "succeeded" ? "badge badge-ok" : "badge badge-bad"}>{result.record.status === "succeeded" ? "成功" : "失败"}</span></div><pre className="action-output"><code>{result.record.output || "命令没有输出"}</code></pre></section>}
        <div className="risk compose-rollback-warning"><AlertTriangle size={18} /><p>部署和回滚只处理 Compose 配置与镜像，不会回滚数据库、上传文件或命名卷。执行前请另行备份业务数据。</p></div>
      </>}
    </div>
    <div className="dialog-footer"><button className="btn" type="button" disabled={Boolean(busy)} onClick={onClose}>关闭</button><button className="btn" type="button" disabled={Boolean(busy) || loading || !projectName.trim() || !workingDirectory.trim() || !content.trim()} onClick={() => void preview()}>{busy === "preview" ? <LoaderCircle className="spin" size={15} /> : <ShieldCheck size={15} />}{busy === "preview" ? "正在校验" : "校验与预检"}</button><button className="btn btn-primary" type="button" disabled={Boolean(busy) || !plan || reviewedSignature !== signature} onClick={() => void deploy()}><Rocket size={15} />{busy === "deploy" ? "正在部署" : "确认部署"}</button></div>
  </dialog>;
}

function ComposePlanView({ plan, stale }: { plan: ComposeDeploymentPlan; stale: boolean }) {
  const labels = { create: "新建", update: "更新", remove: "移除", unchanged: "不变" };
  return <section className="compose-plan"><div className="section-heading"><div><h2>变更预检</h2><span className="table-meta">服务端校验通过 · 配置指纹 {plan.revision ? `${plan.revision.slice(0, 19)}…` : "新项目"}</span></div><span className={stale ? "badge badge-bad" : "badge badge-ok"}>{stale ? "需重新预检" : "已复核"}</span></div><div className="compose-plan-stats"><span>新建 {plan.createCount}</span><span>更新 {plan.updateCount}</span><span>移除 {plan.removeCount}</span><span>不变 {plan.unchangedCount}</span></div><div className="table-wrap"><table className="data-table"><thead><tr><th>服务</th><th>变化</th><th>当前镜像</th><th>目标镜像</th></tr></thead><tbody>{plan.services.map((service) => <tr key={service.service}><td><span className="table-name">{service.service}</span></td><td><span className={service.change === "remove" ? "badge badge-bad" : service.change === "unchanged" ? "badge" : "badge badge-ok"}>{labels[service.change]}</span></td><td><code className="compose-image">{service.currentImage ?? "—"}</code></td><td><code className="compose-image">{service.desiredImage ?? "—"}</code></td></tr>)}</tbody></table></div></section>;
}

interface HistoryProps {
  server: Server;
  project: ComposeProject;
  trusted: TrustedExecutor;
  onClose: () => void;
  onChanged: () => void;
}

export function ComposeDeploymentHistoryView({ server, project, trusted, onClose, onChanged }: HistoryProps) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [history, setHistory] = useState<ComposeDeploymentHistory>();
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [result, setResult] = useState<ComposeDeploymentResult>();

  const load = async () => {
    setError("");
    try { setHistory(await trusted(server, (allow) => api.listComposeDeployments(server.id, project.name, allow))); }
    catch (reason) { setError(errorMessage(reason)); }
  };
  useEffect(() => {
    dialogRef.current?.showModal();
    void load();
  }, []);
  const rollback = async (deploymentId: string, imageCount: number) => {
    if (!history) return;
    if (!window.confirm(`确认恢复这条记录之前的 Compose 配置？\n\n${imageCount > 0 ? `将尽量固定到快照中的 ${imageCount} 个本地镜像摘要；镜像被清理时可能失败。` : "快照没有可用镜像摘要，只恢复配置，标签可能指向更新镜像。"}\n数据库和命名卷不会回滚。`)) return;
    setBusy(deploymentId);
    setError("");
    setResult(undefined);
    try {
      const output = await trusted(server, (allow) => api.rollbackComposeDeployment(server.id, project.name, deploymentId, history.currentRevision, allow));
      setResult(output);
      if (output.record.status === "failed") setError("回滚命令执行失败，请检查输出和服务器状态");
      await load();
      onChanged();
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };

  return <dialog ref={dialogRef} className="dialog compose-history-dialog" onCancel={(event) => { event.preventDefault(); if (!busy) onClose(); }}>
    <div className="dialog-header"><div><h2>{project.name} · 部署历史</h2><span className="table-meta">最多显示 50 条，快照保存在远程项目目录</span></div><button className="icon-btn" type="button" aria-label="关闭部署历史" disabled={Boolean(busy)} onClick={onClose}><X size={18} /></button></div>
    <div className="dialog-body">
      {error && <div className="error-banner" role="alert">{error}</div>}
      {!history && !error && <div className="skeleton compose-editor-skeleton" />}
      {history?.records.length === 0 && <div className="empty"><div><History size={30} /><h2>还没有部署记录</h2><p>从发布编辑器完成首次部署后，这里会显示配置快照与结果。</p></div></div>}
      <div className="compose-history-list">{history?.records.map((record) => <details className="compose-history-record" key={record.id} open={result?.record.id === record.id}><summary><span className={record.status === "succeeded" ? "badge badge-ok" : "badge badge-bad"}>{record.action === "rollback" ? "回滚" : "部署"} · {record.status === "succeeded" ? "成功" : "失败"}</span><span><strong>{formatTime(record.createdAt)}</strong><small>{record.operator} · {record.id.slice(0, 8)}</small></span><span className="compose-history-summary">{record.changes.filter((change) => change.change !== "unchanged").length} 项变化</span></summary><div className="compose-history-detail"><div className="compose-record-facts"><span>配置快照：{record.snapshotAvailable ? "可用" : "无上一版"}</span><span>固定镜像：{record.snapshotImageCount}</span><span>部署后镜像：{record.images.length}</span></div>{record.images.length > 0 && <div className="compose-image-snapshots">{record.images.map((image) => <div key={`${record.id}-${image.service}`}><code>{image.service}</code><span>{image.image}</span><small>{image.imageId ? `${image.imageId.slice(0, 19)}…` : "无摘要"}</small></div>)}</div>}<pre className="action-output"><code>{record.output || "命令没有输出"}</code></pre><button className="btn" type="button" disabled={Boolean(busy) || !record.snapshotAvailable} onClick={() => void rollback(record.id, record.snapshotImageCount)}>{busy === record.id ? <LoaderCircle className="spin" size={15} /> : <RotateCcw size={15} />}{record.snapshotAvailable ? "恢复到此操作前的快照" : "首次部署无法回滚"}</button></div></details>)}</div>
      {result && result.record.status === "succeeded" && <div className="file-notice" role="status"><CheckCircle2 size={16} />回滚已完成，新记录 {result.record.id}</div>}
      <div className="risk compose-rollback-warning"><AlertTriangle size={18} /><p>回滚不会恢复数据库内容、数据卷或被删除的业务文件。固定镜像依赖 VPS 上仍保留对应镜像。</p></div>
    </div>
    <div className="dialog-footer"><button className="btn" type="button" disabled={Boolean(busy)} onClick={() => void load()}>刷新</button><button className="btn btn-primary" type="button" disabled={Boolean(busy)} onClick={onClose}>完成</button></div>
  </dialog>;
}
