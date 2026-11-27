import { useCallback, useEffect, useRef, useState } from "react";
import { ArchiveRestore, CheckCircle2, Database, FolderArchive, HardDrive, History, KeyRound, LoaderCircle, Plus, RefreshCw, ShieldCheck, Trash2, X } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import { formatTime } from "../lib/format";
import type { DataProtectionInput, DataProtectionOverview, DataProtectionPolicy, DataProtectionRun, ResticSnapshot, Server, TrustedExecutor } from "../types";

const emptyInput: DataProtectionInput = {
  name: "", repositoryKind: "local", repository: "/srv/restic/controlvps", sourceKind: "directory",
  paths: ["/etc", "/var/www"], keepLast: 7, scheduleEnabled: false, scheduleIntervalMinutes: 1440,
};
const sourceLabels = { directory: "远程目录", docker_volume: "Docker 卷", postgresql: "PostgreSQL", mysql: "MySQL / MariaDB" };

function formatInterval(minutes: number) {
  if (minutes >= 1440 && minutes % 1440 === 0) return `每 ${minutes / 1440} 天`;
  if (minutes >= 60 && minutes % 60 === 0) return `每 ${minutes / 60} 小时`;
  return `每 ${minutes} 分钟`;
}

export function DataProtectionManager({ server, trusted }: { server: Server; trusted: TrustedExecutor }) {
  const [overview, setOverview] = useState<DataProtectionOverview>();
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [editorOpen, setEditorOpen] = useState(false);
  const [editing, setEditing] = useState<DataProtectionPolicy>();
  const [input, setInput] = useState<DataProtectionInput>(emptyInput);
  const [selected, setSelected] = useState<DataProtectionPolicy>();
  const [snapshots, setSnapshots] = useState<ResticSnapshot[]>([]);
  const [runs, setRuns] = useState<DataProtectionRun[]>([]);
  const [restoreTarget, setRestoreTarget] = useState("");
  const editorRef = useRef<HTMLDialogElement>(null);
  const detailRef = useRef<HTMLDialogElement>(null);

  const load = useCallback(async () => {
    setBusy("load"); setError("");
    try { setOverview(await trusted(server, (allow) => api.dataProtectionOverview(server.id, allow))); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  }, [server, trusted]);
  useEffect(() => { void load(); }, [load]);
  useEffect(() => {
    if (editorOpen && !editorRef.current?.open) editorRef.current?.showModal();
    if (!editorOpen && editorRef.current?.open) editorRef.current.close();
  }, [editorOpen]);
  useEffect(() => {
    if (selected && !detailRef.current?.open) detailRef.current?.showModal();
    if (!selected && detailRef.current?.open) detailRef.current.close();
  }, [selected]);

  const openEditor = (policy?: DataProtectionPolicy) => {
    setEditing(policy);
    setInput(policy ? {
      name: policy.name, repositoryKind: policy.repositoryKind, repository: policy.repository,
      sourceKind: policy.sourceKind, paths: policy.paths, volumeName: policy.volumeName ?? "",
      databaseContainer: policy.databaseContainer ?? "", databaseName: policy.databaseName ?? "",
      databaseUser: policy.databaseUser ?? "", keepLast: policy.keepLast,
      scheduleEnabled: policy.scheduleEnabled, scheduleIntervalMinutes: policy.scheduleIntervalMinutes || 1440,
    } : emptyInput);
    setError(""); setEditorOpen(true);
  };
  const save = async () => {
    setBusy("save"); setError("");
    try {
      if (editing) await api.updateDataProtectionPolicy(editing.id, input);
      else await api.createDataProtectionPolicy(server.id, input);
      setEditorOpen(false); setNotice("数据保护策略已保存，密钥仅写入系统钥匙串"); await load();
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };
  const operate = async (policy: DataProtectionPolicy, action: "initialize" | "check" | "backup") => {
    const messages = {
      initialize: "确认初始化这个 Restic 仓库？已有正常仓库不要重复初始化。",
      check: "确认检查仓库完整性？大数据仓库的检查可能耗时较长。",
      backup: `确认立即备份“${policy.name}”？成功后将按策略保留最近 ${policy.keepLast} 个快照并清理旧快照。`,
    };
    if (!window.confirm(messages[action])) return;
    setBusy(`${action}-${policy.id}`); setError(""); setNotice("");
    try {
      const run = await trusted(server, (allow) => action === "initialize"
        ? api.initializeDataProtection(policy.id, allow)
        : action === "check" ? api.checkDataProtection(policy.id, allow) : api.runDataProtectionBackup(policy.id, allow));
      setNotice(action === "backup" ? `备份成功，快照 ID：${run.snapshotId ?? "未返回"}` : action === "check" ? "仓库完整性检查通过" : "Restic 仓库初始化成功");
      if (selected?.id === policy.id) await openDetails(policy);
    } catch (reason) { setError(`${errorMessage(reason)}。失败记录已保存在策略历史中。`); }
    finally { setBusy(""); }
  };
  const remove = async (policy: DataProtectionPolicy) => {
    if (!window.confirm("确认删除本地策略？Restic 仓库和已有快照不会删除，Keychain 中对应密钥会移除。")) return;
    setBusy(`delete-${policy.id}`); setError("");
    try { await api.deleteDataProtectionPolicy(policy.id); setSelected(undefined); setNotice("本地策略与 Keychain 密钥已删除，远程快照未改动"); await load(); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };
  const openDetails = async (policy: DataProtectionPolicy) => {
    setBusy(`details-${policy.id}`); setError("");
    try {
      const [snapshotList, runList] = await Promise.all([
        trusted(server, (allow) => api.dataProtectionSnapshots(policy.id, allow)),
        api.dataProtectionRuns(policy.id),
      ]);
      setSnapshots(snapshotList); setRuns(runList); setSelected(policy); setRestoreTarget("");
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };
  const restore = async (snapshot: ResticSnapshot) => {
    if (!selected) return;
    const targetDescription = restoreTarget.trim() || "系统自动生成的新 /tmp/controlvps-restore-* 目录";
    if (!window.confirm(`确认把快照 ${snapshot.shortId} 恢复到 ${selected.name} 所在 VPS？\n\n目标：${targetDescription}\n目标必须不存在。不会覆盖原目录、Docker 卷或导入数据库。`)) return;
    setBusy(`restore-${snapshot.id}`); setError(""); setNotice("");
    try {
      const result = await trusted(server, (allow) => api.restoreDataProtectionSnapshot(selected.id, snapshot.id, restoreTarget.trim() || undefined, allow));
      setNotice(`恢复完成：${result.target}。请核对文件权限和业务内容后再手动接管。`);
      setRuns(await api.dataProtectionRuns(selected.id));
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };

  return <div className="protection-manager">
    <div className="panel-header"><div><h2>VPS 数据保护</h2><span className="table-meta">{overview?.resticVersion || "正在检测 Restic"} · 标准 Restic 快照</span></div><div className="heading-actions"><button className="icon-btn" type="button" title="刷新" aria-label="刷新数据保护" disabled={busy !== ""} onClick={() => void load()}><RefreshCw size={16} /></button><button className="btn btn-primary" type="button" disabled={busy !== ""} onClick={() => openEditor()}><Plus size={15} />新建策略</button></div></div>
    <div className="risk flex items-start gap-3"><ShieldCheck size={19} /><p>本地仓库仍位于同一台 VPS，无法抵御整机丢失；生产数据建议使用 S3 兼容异地仓库。恢复始终写入新目录，不自动覆盖目录、卷或数据库。</p></div>
    {overview?.resticVersion.includes("未安装") && <div className="error-banner m-4" role="alert"><p>服务器未安装 Restic。请人工安装后再初始化，例如 Debian/Ubuntu 使用 `apt-get install restic`，Alpine 使用 `apk add restic`。ControlVPS 不会自动安装。</p></div>}
    {error && <div className="error-banner m-4" role="alert">{error}</div>}
    {notice && <div className="file-notice docker-notice" role="status">{notice}</div>}
    {busy === "load" && !overview ? <div className="skeleton docker-skeleton" /> : overview?.policies.length === 0 ? <div className="empty"><div><FolderArchive size={31} /><h2>还没有数据保护策略</h2><p>选择目录、Docker 卷或容器数据库，备份到本地路径或 S3 兼容仓库。</p><button className="btn btn-primary" type="button" onClick={() => openEditor()}><Plus size={15} />新建策略</button></div></div> : <div className="protection-grid">{overview?.policies.map((policy) => <article className="panel protection-card" key={policy.id}><div className="panel-header"><div className="batch-result-title">{policy.sourceKind === "directory" ? <FolderArchive size={20} /> : policy.sourceKind === "docker_volume" ? <HardDrive size={20} /> : <Database size={20} />}<div><h3>{policy.name}</h3><span className="table-meta">{sourceLabels[policy.sourceKind]} · 保留 {policy.keepLast} 个快照</span></div></div><div className="row-actions">{policy.scheduleEnabled && <span className={policy.lastStatus === "failed" ? "badge badge-bad" : "badge badge-ok"}>定时 · {formatInterval(policy.scheduleIntervalMinutes)}</span>}<span className="badge">{policy.repositoryKind === "local" ? "本机仓库" : "S3 异地仓库"}</span></div></div><div className="panel-body"><dl className="facts"><div><dt>备份来源</dt><dd>{policy.sourceKind === "directory" ? policy.paths.join("，") : policy.sourceKind === "docker_volume" ? policy.volumeName : `${policy.databaseContainer} / ${policy.databaseName}`}</dd></div><div><dt>Restic 仓库</dt><dd><code>{policy.repository}</code></dd></div><div><dt>凭据</dt><dd><KeyRound size={13} /> 钥匙串已保存{policy.repositoryKind === "s3" ? "仓库密码与 S3 密钥" : "仓库密码"}</dd></div>{policy.scheduleEnabled && <div><dt>定时计划</dt><dd>{policy.nextRunAt ? `下次 ${formatTime(policy.nextRunAt)}` : "等待调度"}{policy.lastRunAt ? ` · 上次 ${formatTime(policy.lastRunAt)}` : ""}{policy.lastError ? <span className="text-danger"> · 失败：{policy.lastError}</span> : null}</dd></div>}</dl><div className="row-actions protection-actions"><button className="btn btn-primary" type="button" disabled={busy !== "" || overview.resticVersion.includes("未安装")} onClick={() => void operate(policy, "backup")}>{busy === `backup-${policy.id}` ? <LoaderCircle size={15} className="spin" /> : <FolderArchive size={15} />}立即备份</button><button className="btn" type="button" disabled={busy !== ""} onClick={() => void openDetails(policy)}><History size={15} />快照与历史</button><button className="btn" type="button" disabled={busy !== ""} onClick={() => openEditor(policy)}>编辑</button><button className="btn" type="button" title="初始化仓库" disabled={busy !== ""} onClick={() => void operate(policy, "initialize")}>初始化</button><button className="btn" type="button" title="检查仓库完整性" disabled={busy !== ""} onClick={() => void operate(policy, "check")}>检查</button><button className="icon-btn" type="button" title="删除本地策略" aria-label={`删除 ${policy.name} 策略`} disabled={busy !== ""} onClick={() => void remove(policy)}><Trash2 size={15} /></button></div></div></article>)}</div>}
    <dialog ref={editorRef} className="dialog protection-editor" onCancel={(event) => { event.preventDefault(); if (!busy) setEditorOpen(false); }}>
      <div className="dialog-header"><div><h2>{editing ? "编辑数据保护策略" : "新建数据保护策略"}</h2><span className="table-meta">{server.name} · 使用服务器上的 Restic</span></div><button className="icon-btn" type="button" aria-label="关闭策略编辑器" onClick={() => setEditorOpen(false)}><X size={18} /></button></div>
      <div className="dialog-body"><div className="form-grid"><div className="field"><label htmlFor="protection-name">策略名称</label><input id="protection-name" className="input" value={input.name} onChange={(event) => setInput({ ...input, name: event.target.value })} placeholder="例如：网站目录每日备份" /></div><div className="field"><label htmlFor="protection-source">备份来源</label><select id="protection-source" className="select" value={input.sourceKind} onChange={(event) => setInput({ ...input, sourceKind: event.target.value as DataProtectionInput["sourceKind"] })}>{Object.entries(sourceLabels).map(([value, label]) => <option value={value} key={value}>{label}</option>)}</select></div><div className="field"><label htmlFor="protection-repository-kind">仓库类型</label><select id="protection-repository-kind" className="select" value={input.repositoryKind} onChange={(event) => setInput({ ...input, repositoryKind: event.target.value as DataProtectionInput["repositoryKind"], repository: event.target.value === "local" ? "/srv/restic/controlvps" : "s3:https://s3.amazonaws.com/bucket/controlvps" })}><option value="local">VPS 本地路径</option><option value="s3">S3 兼容对象存储</option></select></div><div className="field"><label htmlFor="protection-keep">保留最近快照</label><input id="protection-keep" className="input" type="number" min={1} max={100} value={input.keepLast} onChange={(event) => setInput({ ...input, keepLast: Number(event.target.value) })} /></div></div>
        <div className="field mt-4"><label htmlFor="protection-repository">Restic 仓库地址</label><input id="protection-repository" className="input" value={input.repository} onChange={(event) => setInput({ ...input, repository: event.target.value })} spellCheck={false} /><p className="field-help">本地使用绝对路径；S3 格式为 `s3:https://endpoint/bucket/可选前缀`。</p></div>
        {input.sourceKind === "directory" && <div className="field mt-4"><label htmlFor="protection-paths">远程目录（每行一个绝对路径）</label><textarea id="protection-paths" className="textarea command-editor" value={input.paths.join("\n")} onChange={(event) => setInput({ ...input, paths: event.target.value.split("\n").map((path) => path.trim()).filter(Boolean) })} spellCheck={false} /></div>}
        {input.sourceKind === "docker_volume" && <div className="field mt-4"><label htmlFor="protection-volume">Docker 卷名称</label><input id="protection-volume" className="input" value={input.volumeName ?? ""} onChange={(event) => setInput({ ...input, volumeName: event.target.value })} placeholder="例如 postgres_data" /><p className="field-help">直接读取卷挂载点；对持续写入的数据库卷，请优先使用数据库一致性转储。</p></div>}
        {(input.sourceKind === "postgresql" || input.sourceKind === "mysql") && <div className="form-section"><h2>容器数据库转储</h2><div className="form-grid"><div className="field"><label htmlFor="protection-container">容器名称或 ID</label><input id="protection-container" className="input" value={input.databaseContainer ?? ""} onChange={(event) => setInput({ ...input, databaseContainer: event.target.value })} /></div><div className="field"><label htmlFor="protection-database">数据库名称</label><input id="protection-database" className="input" value={input.databaseName ?? ""} onChange={(event) => setInput({ ...input, databaseName: event.target.value })} /></div><div className="field"><label htmlFor="protection-user">数据库用户</label><input id="protection-user" className="input" value={input.databaseUser ?? ""} onChange={(event) => setInput({ ...input, databaseUser: event.target.value })} placeholder={input.sourceKind === "mysql" ? "默认 root" : "默认 postgres"} /></div><div className="field"><label htmlFor="protection-db-password">数据库密码（可选）</label><input id="protection-db-password" className="input" type="password" value={input.databasePassword ?? ""} onChange={(event) => setInput({ ...input, databasePassword: event.target.value })} placeholder={editing?.hasDatabasePassword ? "留空保留 Keychain 原值" : "容器已配置免密时可留空"} autoComplete="off" /></div></div><p className="field-help">PostgreSQL 使用 pg_dump；MySQL/MariaDB 使用 mysqldump 的单事务模式。SQL 临时文件备份后立即删除。</p></div>}
        <div className="form-section"><h2>定时自动备份</h2><div className="form-grid"><label className="check-row"><input type="checkbox" checked={Boolean(input.scheduleEnabled)} onChange={(event) => setInput({ ...input, scheduleEnabled: event.target.checked })} />应用运行时按间隔自动备份并清理旧快照</label><div className="field"><label htmlFor="protection-schedule-interval">备份频率</label><select id="protection-schedule-interval" className="select" disabled={!input.scheduleEnabled} value={input.scheduleIntervalMinutes ?? 1440} onChange={(event) => setInput({ ...input, scheduleIntervalMinutes: Number(event.target.value) })}><option value={60}>每 1 小时</option><option value={360}>每 6 小时</option><option value={720}>每 12 小时</option><option value={1440}>每 24 小时</option><option value={4320}>每 3 天</option><option value={10080}>每 7 天</option></select></div></div><p className="field-help">定时备份不自动接受未知主机密钥；执行失败会自动通过「通知中心 2.0」发送告警并在总览待办中提示。</p></div>
        <div className="form-section"><h2>仓库与访问凭据</h2><div className="field"><label htmlFor="protection-password">Restic 仓库密码</label><input id="protection-password" className="input" type="password" value={input.repositoryPassword ?? ""} onChange={(event) => setInput({ ...input, repositoryPassword: event.target.value })} placeholder={editing?.hasRepositoryPassword ? "留空保留钥匙串原值" : "必填；丢失后无法读取快照"} autoComplete="off" /></div>{input.repositoryKind === "s3" && <div className="form-grid mt-4"><div className="field"><label htmlFor="protection-access-key">S3 Access Key</label><input id="protection-access-key" className="input" type="password" value={input.s3AccessKeyId ?? ""} onChange={(event) => setInput({ ...input, s3AccessKeyId: event.target.value })} placeholder={editing?.hasS3Credentials ? "两项都留空则保留原值" : "必填"} autoComplete="off" /></div><div className="field"><label htmlFor="protection-secret-key">S3 Secret Key</label><input id="protection-secret-key" className="input" type="password" value={input.s3SecretAccessKey ?? ""} onChange={(event) => setInput({ ...input, s3SecretAccessKey: event.target.value })} placeholder="必填" autoComplete="off" /></div></div>}<p className="field-help">密钥不会写入本地 SQLite、操作记录或远程策略文件；只保存到系统钥匙串，并在执行时通过标准输入注入。</p></div>
      </div><div className="dialog-footer"><button className="btn" type="button" onClick={() => setEditorOpen(false)}>取消</button><button className="btn btn-primary" type="button" disabled={busy !== "" || !input.name.trim() || !input.repository.trim() || input.keepLast < 1} onClick={() => void save()}>保存策略</button></div>
    </dialog>
    <dialog ref={detailRef} className="dialog protection-detail" onCancel={(event) => { event.preventDefault(); if (!busy) setSelected(undefined); }}>
      <div className="dialog-header"><div><h2>{selected?.name} · 快照与历史</h2><span className="table-meta">快照由 Restic 仓库管理，恢复不会自动接管业务</span></div><button className="icon-btn" type="button" aria-label="关闭快照与历史" onClick={() => setSelected(undefined)}><X size={18} /></button></div>
      <div className="dialog-body"><section><div className="section-heading"><h2>可用快照</h2><span className="table-meta">{snapshots.length} 个</span></div>{snapshots.length === 0 ? <div className="empty"><div><FolderArchive size={28} /><h2>没有快照</h2><p>初始化仓库并完成首次备份后，这里会显示可恢复版本。</p></div></div> : <div className="snapshot-list">{snapshots.map((snapshot) => <article className="snapshot-row" key={snapshot.id}><div><code>{snapshot.shortId}</code><strong>{formatTime(snapshot.time)}</strong><small>{snapshot.hostname} · {snapshot.paths.join("，")}</small></div><button className="btn" type="button" disabled={busy !== ""} onClick={() => void restore(snapshot)}><ArchiveRestore size={15} />恢复到新目录</button></article>)}</div>}<div className="field mt-4"><label htmlFor="restore-target">恢复目标（可选）</label><input id="restore-target" className="input" value={restoreTarget} onChange={(event) => setRestoreTarget(event.target.value)} placeholder="留空自动生成 /tmp/controlvps-restore-UUID" /><p className="field-help">自定义目标必须是不存在的绝对路径；留空会创建 `/tmp/controlvps-restore-UUID`。</p></div></section><section className="mt-6"><div className="section-heading"><h2>执行历史</h2><span className="table-meta">最近 100 条</span></div>{runs.length === 0 ? <p className="field-help">还没有初始化、检查、备份或恢复记录。</p> : <div className="protection-runs">{runs.map((run) => <details key={run.id} open={run.status === "failed"}><summary><span className={run.status === "succeeded" ? "badge badge-ok" : run.status === "failed" ? "badge badge-bad" : "badge badge-cool"}>{{ initialize: "初始化", check: "检查", backup: "备份", restore: "恢复" }[run.action]} · {run.status === "succeeded" ? "成功" : run.status === "failed" ? "失败" : "运行中"}</span><span>{formatTime(run.startedAt)} · {run.durationMs} ms</span>{run.snapshotId && <code>{run.snapshotId.slice(0, 12)}</code>}</summary>{run.error && <div className="error-banner" role="alert">{run.error}</div>}{run.target && <p className="field-help">恢复目标：{run.target}</p>}{run.output && <pre className="action-output"><code>{run.output}</code></pre>}</details>)}</div>}</section></div>
      <div className="dialog-footer"><span className="field-help">{notice && <CheckCircle2 size={14} />} 恢复后请人工核对再接管业务</span><button className="btn" type="button" onClick={() => setSelected(undefined)}>关闭</button></div>
    </dialog>
  </div>;
}
