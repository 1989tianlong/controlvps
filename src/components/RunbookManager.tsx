import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ArrowDown, ArrowUp, BookOpenCheck, CheckCircle2, ChevronRight, Play, Plus, Save, Search, Server as ServerIcon, ShieldAlert, Trash2, X, XCircle } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import type { Runbook, RunbookInput, RunbookResult, Server, TrustedExecutor } from "../types";

const emptyDraft: RunbookInput = {
  name: "",
  description: "",
  steps: [{ name: "检查系统状态", command: "uname -a\nfree -h\ndf -h" }],
};

interface Props {
  servers: Server[];
  trusted: TrustedExecutor;
  onAdd: () => void;
}

export function RunbookManager({ servers, trusted, onAdd }: Props) {
  const [runbooks, setRunbooks] = useState<Runbook[]>([]);
  const [selectedId, setSelectedId] = useState<string>();
  const [draft, setDraft] = useState<RunbookInput>(emptyDraft);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [query, setQuery] = useState("");
  const [concurrency, setConcurrency] = useState(2);
  const [canaryCount, setCanaryCount] = useState(0);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [phase, setPhase] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [reviewOpen, setReviewOpen] = useState(false);
  const [reviewText, setReviewText] = useState("");
  const [result, setResult] = useState<RunbookResult>();
  const reviewRef = useRef<HTMLDialogElement>(null);

  const load = useCallback(async () => {
    setError("");
    try { setRunbooks(await api.listRunbooks()); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setLoading(false); }
  }, []);
  useEffect(() => { void load(); }, [load]);
  useEffect(() => {
    if (reviewOpen && !reviewRef.current?.open) reviewRef.current?.showModal();
    if (!reviewOpen && reviewRef.current?.open) reviewRef.current.close();
  }, [reviewOpen]);
  useEffect(() => {
    setSelectedIds((current) => new Set([...current].filter((id) => servers.some((server) => server.id === id))));
  }, [servers]);

  const selected = runbooks.find((runbook) => runbook.id === selectedId);
  const targets = useMemo(() => servers.filter((server) => selectedIds.has(server.id)), [servers, selectedIds]);
  const filtered = servers.filter((server) => `${server.name} ${server.host} ${server.groupName ?? ""}`.toLowerCase().includes(query.toLowerCase()));
  const dangerous = draft.steps.some((step) => /(?:rm\s+-[a-z]*r[a-z]*f|\b(?:reboot|shutdown|mkfs|poweroff)\b|\bdd\s+[^\n]*\bof=)/i.test(step.command));
  const dirty = selected ? JSON.stringify(draft) !== JSON.stringify({ name: selected.name, description: selected.description ?? "", steps: selected.steps.map(({ name, command }) => ({ name, command })) }) : true;

  const select = (runbook: Runbook) => {
    setSelectedId(runbook.id);
    setDraft({ name: runbook.name, description: runbook.description ?? "", steps: runbook.steps.map(({ name, command }) => ({ name, command })) });
    setResult(undefined);
    setError("");
  };
  const createNew = () => {
    setSelectedId(undefined);
    setDraft({ ...emptyDraft, steps: [...emptyDraft.steps] });
    setResult(undefined);
    setError("");
  };
  const save = async () => {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const saved = selectedId ? await api.updateRunbook(selectedId, draft) : await api.createRunbook(draft);
      await load();
      setSelectedId(saved.id);
      setDraft({ name: saved.name, description: saved.description ?? "", steps: saved.steps.map(({ name, command }) => ({ name, command })) });
      setNotice("Runbook 已保存到本机");
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(false); }
  };
  const remove = async () => {
    if (!selectedId || !window.confirm("确认删除这个本地 Runbook？远程服务器不会受到影响。")) return;
    setBusy(true);
    try {
      await api.deleteRunbook(selectedId);
      createNew();
      await load();
      setNotice("Runbook 已删除");
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(false); }
  };
  const updateStep = (index: number, key: "name" | "command", value: string) => {
    setDraft((current) => ({ ...current, steps: current.steps.map((step, stepIndex) => stepIndex === index ? { ...step, [key]: value } : step) }));
  };
  const moveStep = (index: number, offset: number) => {
    setDraft((current) => {
      const target = index + offset;
      if (target < 0 || target >= current.steps.length) return current;
      const steps = [...current.steps];
      [steps[index], steps[target]] = [steps[target], steps[index]];
      return { ...current, steps };
    });
  };
  const toggle = (id: string) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  };
  const openReview = () => {
    setReviewText("");
    setReviewOpen(true);
  };
  const run = async () => {
    if (!selectedId) return;
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
      const effectiveCanary = runTargets.length > 1 ? Math.min(canaryCount, runTargets.length - 1) : 0;
      setPhase(effectiveCanary > 0
        ? `先灰度 ${effectiveCanary} 台；全部通过后，再以最多 ${concurrency} 台并行执行后续批次`
        : `正在执行 ${draft.steps.length} 个步骤，最多 ${concurrency} 台并行`);
      executionStarted = true;
      const output = await api.runRunbook(selectedId, runTargets.map((server) => server.id), concurrency, effectiveCanary);
      setResult(output);
      setNotice(`Runbook 执行完成：${output.successCount} 台成功，${output.failureCount} 台失败${output.skippedCount > 0 ? `，灰度失败已阻止 ${output.skippedCount} 台后续目标` : ""}`);
    } catch (reason) {
      setError(executionStarted
        ? `Runbook 接口失败：${errorMessage(reason)}。部分步骤可能已经执行，请核对操作记录和服务器状态。`
        : `主机身份验证失败：${errorMessage(reason)}。未执行任何 Runbook 步骤。`);
    } finally {
      setPhase("");
      setBusy(false);
    }
  };

  return <>
    {error && <div className="error-banner" role="alert">{error}</div>}
    {notice && <div className="file-notice automation-notice" role="status">{notice}</div>}
    <div className="automation-layout runbook-layout">
      <aside className="panel snippet-library">
        <div className="panel-header"><div><h2>Runbook 库</h2><span className="table-meta">{runbooks.length} 个本地流程</span></div><button className="icon-btn" type="button" aria-label="新建 Runbook" title="新建 Runbook" onClick={createNew} disabled={busy}><Plus size={17} /></button></div>
        {loading ? <div className="skeleton snippet-skeleton" /> : runbooks.length === 0 ? <div className="snippet-empty"><BookOpenCheck size={27} /><p>还没有 Runbook</p><span>把检查、变更和验证组织为有序步骤。</span></div> : <div className="snippet-list">{runbooks.map((runbook) => <button className="snippet-item" data-active={selectedId === runbook.id} type="button" key={runbook.id} disabled={busy} onClick={() => select(runbook)}><span><strong>{runbook.name}</strong><small>{runbook.steps.length} 个步骤 · {runbook.description || "无说明"}</small></span><ChevronRight size={16} /></button>)}</div>}
      </aside>
      <div className="automation-main">
        <section className="panel">
          <div className="panel-header"><div><h2>{selectedId ? "编辑 Runbook" : "新建 Runbook"}</h2><span className="table-meta">按顺序执行，非零退出码会停止当前服务器</span></div><div className="row-actions">{selectedId && <button className="icon-btn" type="button" title="删除 Runbook" aria-label="删除 Runbook" disabled={busy} onClick={() => void remove()}><Trash2 size={16} /></button>}<button className="btn" type="button" disabled={busy || !draft.name.trim() || draft.steps.some((step) => !step.name.trim() || !step.command.trim())} onClick={() => void save()}><Save size={15} />保存 Runbook</button></div></div>
          <div className="panel-body">
            <div className="form-grid"><div className="field"><label htmlFor="runbook-name">流程名称</label><input id="runbook-name" className="input" value={draft.name} disabled={busy} onChange={(event) => setDraft({ ...draft, name: event.target.value })} placeholder="例如：Web 服务安全重启" /></div><div className="field"><label htmlFor="runbook-description">用途与注意事项</label><input id="runbook-description" className="input" value={draft.description} disabled={busy} onChange={(event) => setDraft({ ...draft, description: event.target.value })} placeholder="变更前检查、服务重启、变更后验证" /></div></div>
            <div className="runbook-steps">{draft.steps.map((step, index) => <article className="runbook-step" key={index}><div className="runbook-step-heading"><span className="runbook-step-number">{index + 1}</span><input className="input" aria-label={`步骤 ${index + 1} 名称`} value={step.name} disabled={busy} onChange={(event) => updateStep(index, "name", event.target.value)} /><div className="row-actions"><button className="icon-btn" type="button" aria-label={`上移步骤 ${index + 1}`} disabled={busy || index === 0} onClick={() => moveStep(index, -1)}><ArrowUp size={15} /></button><button className="icon-btn" type="button" aria-label={`下移步骤 ${index + 1}`} disabled={busy || index === draft.steps.length - 1} onClick={() => moveStep(index, 1)}><ArrowDown size={15} /></button><button className="icon-btn" type="button" aria-label={`删除步骤 ${index + 1}`} disabled={busy || draft.steps.length === 1} onClick={() => setDraft({ ...draft, steps: draft.steps.filter((_, stepIndex) => stepIndex !== index) })}><Trash2 size={15} /></button></div></div><textarea className="textarea command-editor" aria-label={`步骤 ${index + 1} 命令`} value={step.command} disabled={busy} onChange={(event) => updateStep(index, "command", event.target.value)} spellCheck={false} /></article>)}</div>
            <div className="runbook-editor-footer"><span>支持 {"{{hostname}}"}、{"{{username}}"}、{"{{name}}"} 变量</span><button className="btn" type="button" disabled={busy || draft.steps.length >= 30} onClick={() => setDraft({ ...draft, steps: [...draft.steps, { name: `步骤 ${draft.steps.length + 1}`, command: "" }] })}><Plus size={15} />添加步骤</button></div>
          </div>
        </section>
        <section className="panel target-panel">
          <div className="panel-header"><div><h2>执行目标</h2><span className="table-meta">已选择 {targets.length} / {servers.length} 台{selectedId && dirty ? " · 有未保存修改" : ""}</span></div><div className="heading-actions"><select className="select" aria-label="Runbook 灰度首批数量" value={targets.length > 1 ? Math.min(canaryCount, targets.length - 1) : 0} disabled={busy || targets.length < 2} onChange={(event) => setCanaryCount(Number(event.target.value))}><option value={0}>不灰度</option><option value={1}>首批灰度 1 台</option><option value={2}>首批灰度 2 台</option><option value={3}>首批灰度 3 台</option></select><select className="select" aria-label="Runbook 并发数" value={concurrency} disabled={busy} onChange={(event) => setConcurrency(Number(event.target.value))}><option value={1}>1 台并行</option><option value={2}>2 台并行</option><option value={3}>3 台并行</option><option value={5}>5 台并行</option></select><button className="btn btn-primary" type="button" disabled={busy || !selectedId || dirty || targets.length === 0} title={dirty ? "请先保存 Runbook 修改" : undefined} onClick={openReview}><Play size={15} />审查并执行</button></div></div>
          <div className="target-filter"><div className="search-wrap"><Search size={16} /><input className="input" type="search" aria-label="搜索 Runbook 执行目标" placeholder="搜索名称、地址或分组" value={query} onChange={(event) => setQuery(event.target.value)} /></div></div>
          {servers.length === 0 ? <div className="empty"><div><ServerIcon size={28} /><h2>先添加 VPS</h2><p>添加服务器后即可执行 Runbook。</p><button className="btn btn-primary" type="button" onClick={onAdd}>添加 VPS</button></div></div> : <div className="target-list">{filtered.map((server) => <label className="target-item" key={server.id}><input type="checkbox" checked={selectedIds.has(server.id)} disabled={busy} onChange={() => toggle(server.id)} /><span><strong>{server.name}</strong><small>{server.username}@{server.host}:{server.port}</small></span><span className="badge">{server.groupName || "未分组"}</span></label>)}</div>}
        </section>
      </div>
    </div>
    {phase && <div className="file-progress automation-phase" role="status">{phase}</div>}
    {result && <section className="batch-results"><div className="page-heading"><div><h2>{result.runbookName} · 执行结果</h2><p>{result.successCount} 台成功，{result.failureCount} 台失败{result.skippedCount > 0 ? `，${result.skippedCount} 台因灰度失败未执行` : ""}，总耗时 {result.durationMs} ms。失败服务器的未执行步骤已跳过。</p></div></div>{result.results.map((server) => <article className="panel batch-result" key={server.serverId}><div className="panel-header"><div className="batch-result-title">{server.success ? <CheckCircle2 size={19} /> : <XCircle size={19} />}<div><h3>{server.serverName}</h3><span className="table-meta">已执行 {server.steps.length} / {draft.steps.length} 步 · {server.durationMs} ms</span></div></div><span className={server.success ? "badge badge-ok" : "badge badge-bad"}>{server.success ? "完成" : "未完成"}</span></div>{server.error && <div className="error-banner m-4" role="alert">{server.error}</div>}<div className="runbook-results">{server.steps.map((step, index) => <details key={step.stepId} open={!step.success}><summary><span>{index + 1}. {step.stepName}</span><span className={step.success ? "badge badge-ok" : "badge badge-bad"}>{step.exitCode === null ? "执行失败" : `退出码 ${step.exitCode}`}</span></summary>{step.error && <p className="runbook-step-error">{step.error}</p>}{step.output && <pre className="action-output"><code>{step.output}</code></pre>}<details className="batch-command"><summary>查看实际命令</summary><pre><code>{step.command}</code></pre></details></details>)}</div></article>)}</section>}
    <dialog ref={reviewRef} className="dialog batch-review" onCancel={(event) => { event.preventDefault(); if (!busy) setReviewOpen(false); }}>
      <div className="dialog-header"><h2>审查 Runbook</h2><button className="icon-btn" type="button" aria-label="关闭" onClick={() => setReviewOpen(false)}><X size={18} /></button></div>
      <div className="dialog-body">{dangerous && <div className="risk flex items-start gap-3"><ShieldAlert size={22} /><p>流程包含高风险命令。请输入“执行”确认，并逐行核对目标和命令。</p></div>}<p><strong>{draft.name}</strong> 将在 {targets.length} 台服务器执行 {draft.steps.length} 个步骤。单台失败后不会继续它的后续步骤。{targets.length > 1 && canaryCount > 0 ? `首批 ${Math.min(canaryCount, targets.length - 1)} 台全部成功后才会执行其余目标；首批任何失败都会阻止后续批次。` : "当前未启用灰度，所有目标会直接进入执行队列。"}</p><div className="runbook-review-steps">{draft.steps.map((step, index) => <section key={index}><h3>{index + 1}. {step.name}</h3><pre className="review-command"><code>{step.command}</code></pre></section>)}</div><div className="review-targets">{targets.map((server) => <span className="badge" key={server.id}>{server.name} · {server.host}</span>)}</div>{dangerous && <div className="field mt-4"><label htmlFor="runbook-confirmation">输入“执行”以继续</label><input id="runbook-confirmation" className="input" value={reviewText} onChange={(event) => setReviewText(event.target.value)} autoComplete="off" /></div>}</div>
      <div className="dialog-footer"><button className="btn" type="button" onClick={() => setReviewOpen(false)}>返回编辑</button><button className={dangerous ? "btn btn-danger" : "btn btn-primary"} type="button" disabled={dangerous && reviewText !== "执行"} onClick={() => void run()}><Play size={15} />确认在 {targets.length} 台执行</button></div>
    </dialog>
  </>;
}
