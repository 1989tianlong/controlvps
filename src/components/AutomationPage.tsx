import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { BookOpenCheck, CalendarClock, CheckCircle2, ChevronRight, ListChecks, Play, Plus, Save, Search, Server as ServerIcon, ShieldAlert, TerminalSquare, Trash2, X, XCircle } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import type { BatchResult, Server, Snippet, SnippetInput, TrustedExecutor } from "../types";
import { RunbookManager } from "./RunbookManager";
import { RunbookOperations } from "./RunbookOperations";

const emptyDraft: SnippetInput = { name: "", description: "", command: "" };
const variables = [
  { token: "{{hostname}}", label: "服务器地址" },
  { token: "{{username}}", label: "SSH 用户名" },
  { token: "{{name}}", label: "服务器名称" },
];

interface Props {
  servers: Server[];
  trusted: TrustedExecutor;
  onAdd: () => void;
  requestedMode?: "snippets" | "runbooks" | "operations";
}

export function AutomationPage(props: Props) {
  const [mode, setMode] = useState<"snippets" | "runbooks" | "operations">("snippets");
  useEffect(() => {
    if (props.requestedMode) setMode(props.requestedMode);
  }, [props.requestedMode]);
  return <>
    <div className="page-heading"><div><h1>批量运维</h1><p>保存常用命令，或使用分步 Runbook 执行可审查的运维流程。</p></div>{mode === "runbooks" && <span className="badge badge-cool"><BookOpenCheck size={15} />失败即停止当前服务器后续步骤</span>}</div>
    <div className="workspace-switch" role="tablist" aria-label="运维模式"><button className="tab" type="button" role="tab" aria-selected={mode === "snippets"} onClick={() => setMode("snippets")}><ListChecks size={16} />命令片段</button><button className="tab" type="button" role="tab" aria-selected={mode === "runbooks"} onClick={() => setMode("runbooks")}><BookOpenCheck size={16} />Runbook</button><button className="tab" type="button" role="tab" aria-selected={mode === "operations"} onClick={() => setMode("operations")}><CalendarClock size={16} />计划与历史</button></div>
    {mode === "snippets" ? <SnippetAutomation {...props} /> : mode === "runbooks" ? <RunbookManager {...props} /> : <RunbookOperations {...props} />}
  </>;
}

function SnippetAutomation({ servers, trusted, onAdd }: Props) {
  const [snippets, setSnippets] = useState<Snippet[]>([]);
  const [selectedSnippetId, setSelectedSnippetId] = useState<string>();
  const [draft, setDraft] = useState<SnippetInput>(emptyDraft);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [query, setQuery] = useState("");
  const [concurrency, setConcurrency] = useState(3);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [phase, setPhase] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [reviewOpen, setReviewOpen] = useState(false);
  const [result, setResult] = useState<BatchResult>();
  const reviewRef = useRef<HTMLDialogElement>(null);

  const loadSnippets = useCallback(async () => {
    setError("");
    try { setSnippets(await api.listSnippets()); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setLoading(false); }
  }, []);

  useEffect(() => { void loadSnippets(); }, [loadSnippets]);
  useEffect(() => {
    if (reviewOpen && !reviewRef.current?.open) reviewRef.current?.showModal();
    if (!reviewOpen && reviewRef.current?.open) reviewRef.current.close();
  }, [reviewOpen]);
  useEffect(() => {
    setSelectedIds((current) => new Set([...current].filter((id) => servers.some((server) => server.id === id))));
  }, [servers]);

  const targets = useMemo(() => servers.filter((server) => selectedIds.has(server.id)), [servers, selectedIds]);
  const filtered = servers.filter((server) => `${server.name} ${server.host} ${server.groupName ?? ""}`.toLowerCase().includes(query.toLowerCase()));
  const dangerous = /(?:rm\s+-[a-z]*r[a-z]*f|\b(?:reboot|shutdown|mkfs|poweroff)\b|\bdd\s+[^\n]*\bof=)/i.test(draft.command);

  const selectSnippet = (snippet: Snippet) => {
    setSelectedSnippetId(snippet.id);
    setDraft({ name: snippet.name, description: snippet.description ?? "", command: snippet.command });
    setResult(undefined);
    setError("");
  };
  const newSnippet = () => {
    setSelectedSnippetId(undefined);
    setDraft(emptyDraft);
    setResult(undefined);
    setError("");
  };
  const save = async () => {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const saved = selectedSnippetId
        ? await api.updateSnippet(selectedSnippetId, draft)
        : await api.createSnippet(draft);
      await loadSnippets();
      setSelectedSnippetId(saved.id);
      setDraft({ name: saved.name, description: saved.description ?? "", command: saved.command });
      setNotice("命令片段已保存到本机");
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(false); }
  };
  const removeSnippet = async () => {
    if (!selectedSnippetId || !window.confirm("确认删除这个本地命令片段？远程服务器不会受到影响。")) return;
    setBusy(true);
    setError("");
    try {
      await api.deleteSnippet(selectedSnippetId);
      newSnippet();
      await loadSnippets();
      setNotice("命令片段已删除");
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(false); }
  };
  const toggle = (id: string) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  };
  const toggleFiltered = () => {
    const allSelected = filtered.length > 0 && filtered.every((server) => selectedIds.has(server.id));
    setSelectedIds((current) => {
      const next = new Set(current);
      for (const server of filtered) {
        if (allSelected) next.delete(server.id); else next.add(server.id);
      }
      return next;
    });
  };

  const run = async () => {
    const command = draft.command;
    const runTargets = targets;
    setReviewOpen(false);
    setBusy(true);
    setError("");
    setNotice("");
    setResult(undefined);
    let executionStarted = false;
    try {
      for (const [index, server] of runTargets.entries()) {
        setPhase(`正在验证主机身份 ${index + 1} / ${runTargets.length}：${server.name}`);
        await trusted(server, (allow) => api.testConnection(server.id, allow));
      }
      setPhase(`正在以 ${concurrency} 路并发执行命令`);
      executionStarted = true;
      const output = await api.runBatchCommand(runTargets.map((server) => server.id), command, concurrency);
      setResult(output);
      setPhase("");
      setNotice(`执行完成：${output.successCount} 台成功，${output.failureCount} 台失败，总耗时 ${output.durationMs} ms`);
    } catch (reason) {
      setPhase("");
      setError(executionStarted
        ? `批量任务接口失败：${errorMessage(reason)}。部分服务器可能已经执行，请核对操作记录与服务器状态。`
        : `主机身份验证失败：${errorMessage(reason)}。未执行任何批量命令。`);
    } finally { setBusy(false); }
  };

  return <>
    <div className="section-heading automation-subheading"><span className="table-meta">单条命令可安全转义服务器变量并并发执行</span><button className="btn btn-primary" type="button" onClick={newSnippet} disabled={busy}><Plus size={16} />新建片段</button></div>
    {error && <div className="error-banner" role="alert">{error}</div>}
    {notice && <div className="file-notice automation-notice" role="status">{notice}</div>}
    <div className="automation-layout">
      <aside className="panel snippet-library">
        <div className="panel-header"><div><h2>命令片段</h2><span className="table-meta">{snippets.length} 个本地片段</span></div><TerminalSquare size={18} /></div>
        {loading ? <div className="skeleton snippet-skeleton" /> : snippets.length === 0 ? <div className="snippet-empty"><TerminalSquare size={26} /><p>还没有命令片段</p><span>在右侧编写并保存常用命令。</span></div> : <div className="snippet-list">{snippets.map((snippet) => <button type="button" key={snippet.id} className="snippet-item" data-active={selectedSnippetId === snippet.id} disabled={busy} onClick={() => selectSnippet(snippet)}><span><strong>{snippet.name}</strong><small>{snippet.description || "无说明"}</small></span><ChevronRight size={16} /></button>)}</div>}
      </aside>
      <div className="automation-main">
        <section className="panel">
          <div className="panel-header"><div><h2>{selectedSnippetId ? "编辑命令片段" : "命令编辑器"}</h2><span className="table-meta">变量会作为 Shell 参数安全转义</span></div><div className="row-actions">{selectedSnippetId && <button className="icon-btn" type="button" title="删除片段" aria-label="删除片段" disabled={busy} onClick={() => void removeSnippet()}><Trash2 size={16} /></button>}<button className="btn" type="button" disabled={busy || !draft.name.trim() || !draft.command.trim()} onClick={() => void save()}><Save size={15} />保存片段</button></div></div>
          <div className="panel-body">
            <div className="form-grid"><div className="field"><label htmlFor="snippet-name">片段名称</label><input id="snippet-name" className="input" value={draft.name} disabled={busy} onChange={(event) => setDraft({ ...draft, name: event.target.value })} placeholder="例如：查看磁盘使用" /></div><div className="field"><label htmlFor="snippet-description">说明</label><input id="snippet-description" className="input" value={draft.description} disabled={busy} onChange={(event) => setDraft({ ...draft, description: event.target.value })} placeholder="用途和注意事项" /></div></div>
            <div className="field mt-4"><label htmlFor="snippet-command">Shell 命令</label><textarea id="snippet-command" className="textarea command-editor" value={draft.command} disabled={busy} onChange={(event) => setDraft({ ...draft, command: event.target.value })} placeholder={"printf 'host=%s user=%s\\n' {{hostname}} {{username}}\ndf -h"} spellCheck={false} /></div>
            <div className="variable-row"><span>插入变量</span>{variables.map((variable) => <button className="badge" type="button" key={variable.token} title={variable.label} disabled={busy} onClick={() => setDraft({ ...draft, command: `${draft.command}${draft.command && !draft.command.endsWith(" ") ? " " : ""}${variable.token}` })}>{variable.token}</button>)}</div>
          </div>
        </section>
        <section className="panel target-panel">
          <div className="panel-header"><div><h2>执行目标</h2><span className="table-meta">已选择 {targets.length} / {servers.length} 台</span></div><div className="heading-actions"><select className="select" aria-label="批量执行并发数" value={concurrency} disabled={busy} onChange={(event) => setConcurrency(Number(event.target.value))}><option value={1}>1 路并发</option><option value={3}>3 路并发</option><option value={5}>5 路并发</option><option value={10}>10 路并发</option></select><button className="btn btn-primary" type="button" disabled={busy || !draft.command.trim() || targets.length === 0} onClick={() => setReviewOpen(true)}><Play size={15} />检查并执行</button></div></div>
          <div className="target-filter"><div className="search-wrap"><Search size={16} /><input className="input" type="search" aria-label="搜索执行目标" placeholder="搜索名称、地址或分组" value={query} onChange={(event) => setQuery(event.target.value)} /></div><button className="btn" type="button" disabled={busy || filtered.length === 0} onClick={toggleFiltered}>{filtered.every((server) => selectedIds.has(server.id)) && filtered.length > 0 ? "取消当前筛选" : "选择当前筛选"}</button></div>
          {servers.length === 0 ? <div className="empty"><div><ServerIcon size={28} /><h2>先添加 VPS</h2><p>添加服务器后即可创建批量运维任务。</p><button className="btn btn-primary" type="button" onClick={onAdd}>添加 VPS</button></div></div> : <div className="target-list">{filtered.map((server) => <label className="target-item" key={server.id}><input type="checkbox" checked={selectedIds.has(server.id)} disabled={busy} onChange={() => toggle(server.id)} /><span><strong>{server.name}</strong><small>{server.username}@{server.host}:{server.port}</small></span><span className="badge">{server.groupName || "未分组"}</span></label>)}</div>}
        </section>
      </div>
    </div>
    {phase && <div className="file-progress automation-phase" role="status">{phase}</div>}
    {result && <section className="batch-results"><div className="page-heading"><div><h2>本次执行结果</h2><p>{result.successCount} 台成功，{result.failureCount} 台失败，总耗时 {result.durationMs} ms。</p></div></div>{result.results.map((item) => <article className="panel batch-result" key={item.serverId}><div className="panel-header"><div className="batch-result-title">{item.success ? <CheckCircle2 size={19} /> : <XCircle size={19} />}<div><h3>{item.serverName}</h3><span className="table-meta">{item.exitCode === null ? "连接或执行失败" : `退出码 ${item.exitCode}`} · {item.durationMs} ms</span></div></div><span className={item.success ? "badge badge-ok" : "badge badge-bad"}>{item.success ? "成功" : "失败"}</span></div>{item.error && <div className="error-banner m-4" role="alert">{item.error}</div>}{item.output && <pre className="action-output batch-output"><code>{item.output}</code></pre>}{!item.output && !item.error && <p className="batch-no-output">命令没有输出</p>}<details className="batch-command"><summary>查看本机实际执行命令</summary><pre><code>{item.command}</code></pre></details></article>)}</section>}
    <dialog ref={reviewRef} className="dialog batch-review" onCancel={(event) => { event.preventDefault(); if (!busy) setReviewOpen(false); }}>
      <div className="dialog-header"><h2>确认批量执行</h2><button className="icon-btn" type="button" aria-label="关闭" onClick={() => setReviewOpen(false)}><X size={18} /></button></div>
      <div className="dialog-body">{dangerous && <div className="risk flex items-start gap-3"><ShieldAlert size={22} /><p>命令包含删除、关机或磁盘写入等高风险特征，请逐行检查。</p></div>}<p>即将在 <strong>{targets.length} 台服务器</strong>上以 {concurrency} 路并发执行。执行前会逐台验证 SSH 主机身份，任何验证失败都会中止任务。</p><pre className="review-command"><code>{draft.command}</code></pre><div className="review-targets">{targets.map((server) => <span className="badge" key={server.id}>{server.name} · {server.host}</span>)}</div></div>
      <div className="dialog-footer"><button className="btn" type="button" onClick={() => setReviewOpen(false)}>返回编辑</button><button className={dangerous ? "btn btn-danger" : "btn btn-primary"} type="button" onClick={() => void run()}><Play size={15} />确认在 {targets.length} 台执行</button></div>
    </dialog>
  </>;
}
