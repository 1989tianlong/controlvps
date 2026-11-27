import { useCallback, useEffect, useRef, useState } from "react";
import { CalendarClock, CheckCircle2, Clock3, History, LoaderCircle, Play, Plus, RefreshCw, RotateCcw, ShieldAlert, Square, Trash2, X, XCircle } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import { formatTime } from "../lib/format";
import type { Runbook, RunbookExecution, RunbookExecutionStatus, RunbookSchedule, RunbookScheduleInput, Server, TrustedExecutor } from "../types";

const statusLabels: Record<RunbookExecutionStatus, string> = {
  queued: "等待执行", running: "执行中", succeeded: "成功", failed: "失败",
  partial: "部分成功", cancelled: "已取消", timed_out: "已超时", interrupted: "已中断",
};
const active = (execution: RunbookExecution) => execution.status === "queued" || execution.status === "running";
const emptyInput: RunbookScheduleInput = { runbookId: "", serverIds: [], intervalMinutes: 60, enabled: true, concurrency: 2, canaryCount: 0, timeoutSeconds: 300 };

export function RunbookOperations({ servers, trusted, onAdd }: { servers: Server[]; trusted: TrustedExecutor; onAdd: () => void }) {
  const [runbooks, setRunbooks] = useState<Runbook[]>([]);
  const [schedules, setSchedules] = useState<RunbookSchedule[]>([]);
  const [executions, setExecutions] = useState<RunbookExecution[]>([]);
  const [editor, setEditor] = useState<RunbookSchedule>();
  const [editorOpen, setEditorOpen] = useState(false);
  const [input, setInput] = useState<RunbookScheduleInput>(emptyInput);
  const [detail, setDetail] = useState<RunbookExecution>();
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [loading, setLoading] = useState(true);
  const editorRef = useRef<HTMLDialogElement>(null);
  const detailRef = useRef<HTMLDialogElement>(null);

  const load = useCallback(async (quiet = false) => {
    if (!quiet) setError("");
    try {
      const [bookList, scheduleList, executionList] = await Promise.all([api.listRunbooks(), api.runbookSchedules(), api.runbookExecutions()]);
      setRunbooks(bookList);
      setSchedules(scheduleList);
      setExecutions(executionList);
    } catch (reason) { if (!quiet) setError(errorMessage(reason)); }
    finally { setLoading(false); }
  }, []);
  const hasActiveExecution = executions.some(active);
  const activeDetailId = detail && active(detail) ? detail.id : undefined;
  useEffect(() => { void load(); }, [load]);
  useEffect(() => {
    if (!hasActiveExecution && !activeDetailId) return;
    const timer = window.setInterval(() => {
      void load(true);
      if (activeDetailId) void api.runbookExecution(activeDetailId).then(setDetail).catch(() => undefined);
    }, 1200);
    return () => window.clearInterval(timer);
  }, [hasActiveExecution, activeDetailId, load]);
  useEffect(() => {
    if (editorOpen && !editorRef.current?.open) editorRef.current?.showModal();
    if (!editorOpen && editorRef.current?.open) editorRef.current.close();
  }, [editorOpen]);
  useEffect(() => {
    if (detail && !detailRef.current?.open) detailRef.current?.showModal();
    if (!detail && detailRef.current?.open) detailRef.current.close();
  }, [detail]);

  const openEditor = (schedule?: RunbookSchedule) => {
    setEditor(schedule);
    setInput(schedule ? {
      runbookId: schedule.runbookId, serverIds: schedule.serverIds, intervalMinutes: schedule.intervalMinutes,
      enabled: schedule.enabled, concurrency: schedule.concurrency, canaryCount: schedule.canaryCount,
      timeoutSeconds: schedule.timeoutSeconds,
    } : { ...emptyInput, runbookId: runbooks[0]?.id ?? "" });
    setError("");
    setEditorOpen(true);
  };
  const save = async () => {
    setBusy("save"); setError("");
    try {
      if (editor) await api.updateRunbookSchedule(editor.id, input);
      else await api.createRunbookSchedule(input);
      setEditorOpen(false); setNotice("计划任务已保存；下次执行时间已按新间隔计算"); await load();
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };
  const trustTargets = async (ids: string[]) => {
    for (const id of ids) {
      const server = servers.find((item) => item.id === id);
      if (!server) throw new Error("目标 VPS 已不存在，请先编辑计划任务");
      await trusted(server, (allow) => api.testConnection(server.id, allow));
    }
  };
  const runNow = async (schedule: RunbookSchedule) => {
    setBusy(`run-${schedule.id}`); setError(""); setNotice("");
    try {
      await trustTargets(schedule.serverIds);
      const execution = await api.runRunbookScheduleNow(schedule.id);
      setNotice(`已启动 ${schedule.runbookName}，执行 ID：${execution.id.slice(0, 8)}`);
      setDetail(execution); await load(true);
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };
  const toggle = async (schedule: RunbookSchedule) => {
    setBusy(`toggle-${schedule.id}`); setError("");
    try { await api.setRunbookScheduleEnabled(schedule.id, !schedule.enabled); await load(); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };
  const remove = async (schedule: RunbookSchedule) => {
    if (!window.confirm("确认删除这个计划任务？已有执行历史会保留，正在运行的任务不会被取消。")) return;
    setBusy(`delete-${schedule.id}`); setError("");
    try { await api.deleteRunbookSchedule(schedule.id); setNotice("计划任务已删除"); await load(); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };
  const cancel = async (execution: RunbookExecution) => {
    if (!window.confirm("确认取消这个任务？当前 SSH 通道会关闭，远端命令是否立即停止需要到服务器核对。")) return;
    setBusy(`cancel-${execution.id}`); setError("");
    try { setDetail(await api.cancelRunbookExecution(execution.id)); setNotice("取消请求已发送"); await load(true); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };
  const retry = async (execution: RunbookExecution) => {
    const successful = new Set(execution.result?.results.filter((item) => item.success).map((item) => item.serverId) ?? []);
    const ids = execution.targetIds.filter((id) => !successful.has(id));
    setBusy(`retry-${execution.id}`); setError("");
    try {
      await trustTargets(ids);
      const retried = await api.retryRunbookExecution(execution.id);
      setDetail(retried); setNotice("已使用历史 Runbook 快照重试失败目标"); await load(true);
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };
  const openDetail = async (execution: RunbookExecution) => {
    setBusy(`detail-${execution.id}`); setError("");
    try { setDetail(await api.runbookExecution(execution.id)); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };

  return <>
    <div className="risk flex items-start gap-3"><ShieldAlert size={19} /><p>计划任务在 ControlVPS 运行时执行，退出 App 后暂停；重新打开后到期任务只启动一次。自动执行不会接受未知或已变化的 SSH 主机密钥，也不会在弹窗中请求密码。</p></div>
    {error && <div className="error-banner" role="alert">{error}</div>}
    {notice && <div className="file-notice automation-notice" role="status">{notice}</div>}
    <section className="panel">
      <div className="panel-header"><div><h2>Runbook 计划任务</h2><span className="table-meta">{schedules.length} 个任务 · 本地调度 · 防止重叠执行</span></div><div className="heading-actions"><button className="icon-btn" type="button" title="刷新" aria-label="刷新计划与历史" onClick={() => void load()}><RefreshCw size={16} /></button><button className="btn btn-primary" type="button" disabled={runbooks.length === 0 || servers.length === 0} onClick={() => openEditor()}><Plus size={15} />新建计划</button></div></div>
      {loading ? <div className="skeleton docker-skeleton" /> : schedules.length === 0 ? <div className="empty"><div><CalendarClock size={30} /><h2>还没有计划任务</h2><p>{runbooks.length === 0 ? "先在 Runbook 页创建并保存流程。" : "选择已保存的 Runbook 和固定目标，按间隔自动执行。"}</p>{servers.length === 0 && <button className="btn btn-primary" type="button" onClick={onAdd}>添加 VPS</button>}</div></div> : <div className="table-wrap"><table className="data-table"><thead><tr><th>流程与目标</th><th>执行策略</th><th>下次执行</th><th>状态</th><th className="text-right">操作</th></tr></thead><tbody>{schedules.map((schedule) => <tr key={schedule.id}><td><span className="table-name">{schedule.runbookName}</span><span className="table-meta">{schedule.serverIds.length} 台目标</span></td><td>每 {schedule.intervalMinutes < 60 ? `${schedule.intervalMinutes} 分钟` : `${schedule.intervalMinutes / 60} 小时`} · {schedule.concurrency} 路并发<br /><span className="table-meta">单步 {schedule.timeoutSeconds} 秒 · 灰度 {schedule.canaryCount} 台</span></td><td>{schedule.enabled && schedule.nextRunAt ? formatTime(schedule.nextRunAt) : "已暂停"}<br /><span className="table-meta">{schedule.lastRunAt ? `上次 ${formatTime(schedule.lastRunAt)}` : "尚未执行"}</span></td><td><button className={schedule.enabled ? "badge badge-ok" : "badge"} type="button" disabled={busy !== ""} onClick={() => void toggle(schedule)}>{schedule.enabled ? "已启用" : "已暂停"}</button>{schedule.lastError && <span className="table-meta text-danger">{schedule.lastError}</span>}</td><td><div className="row-actions justify-end"><button className="btn" type="button" disabled={busy !== ""} onClick={() => void runNow(schedule)}><Play size={14} />立即执行</button><button className="btn" type="button" disabled={busy !== ""} onClick={() => openEditor(schedule)}>编辑</button><button className="icon-btn" type="button" title="删除计划" aria-label={`删除 ${schedule.runbookName} 计划`} disabled={busy !== ""} onClick={() => void remove(schedule)}><Trash2 size={15} /></button></div></td></tr>)}</tbody></table></div>}
    </section>
    <section className="panel mt-5">
      <div className="panel-header"><div><h2>持久化执行历史</h2><span className="table-meta">保留最近 100 条，包含手动、计划和重试执行</span></div><History size={19} /></div>
      {executions.length === 0 ? <div className="empty"><div><History size={28} /><h2>还没有执行记录</h2><p>从 Runbook 或计划任务启动后，可在这里查看结果、取消或重试。</p></div></div> : <div className="execution-list">{executions.map((execution) => <article className="execution-row" key={execution.id}><button className="execution-main" type="button" onClick={() => void openDetail(execution)}><span className={execution.status === "succeeded" ? "badge badge-ok" : active(execution) ? "badge badge-cool" : "badge badge-bad"}>{active(execution) && <LoaderCircle size={12} className="spin" />}{statusLabels[execution.status]}</span><span><strong>{execution.runbookName}</strong><small>{formatTime(execution.createdAt)} · {{ manual: "手动", scheduled: "计划", retry: "重试" }[execution.trigger]} · {execution.targetIds.length} 台目标</small></span><span className="table-meta">{execution.successCount} 成功 / {execution.failureCount} 失败 / {execution.skippedCount} 跳过</span></button><div className="row-actions">{active(execution) && <button className="icon-btn" type="button" title="取消执行" aria-label={`取消 ${execution.runbookName}`} disabled={busy !== ""} onClick={() => void cancel(execution)}><Square size={15} /></button>}{!active(execution) && (execution.failureCount > 0 || execution.skippedCount > 0 || execution.error) && <button className="icon-btn" type="button" title="重试失败目标" aria-label={`重试 ${execution.runbookName}`} disabled={busy !== ""} onClick={() => void retry(execution)}><RotateCcw size={15} /></button>}</div></article>)}</div>}
    </section>
    <dialog ref={editorRef} className="dialog schedule-dialog" onCancel={(event) => { event.preventDefault(); if (!busy) setEditorOpen(false); }}>
      <div className="dialog-header"><div><h2>{editor ? "编辑计划任务" : "新建计划任务"}</h2><span className="table-meta">保存 Runbook 当前版本作为每次启动时的执行来源</span></div><button className="icon-btn" type="button" aria-label="关闭计划编辑器" onClick={() => setEditorOpen(false)}><X size={18} /></button></div>
      <div className="dialog-body"><div className="form-grid"><div className="field"><label htmlFor="schedule-runbook">Runbook</label><select id="schedule-runbook" className="select" value={input.runbookId} onChange={(event) => setInput({ ...input, runbookId: event.target.value })}>{runbooks.map((runbook) => <option value={runbook.id} key={runbook.id}>{runbook.name} · {runbook.steps.length} 步</option>)}</select></div><div className="field"><label htmlFor="schedule-interval">执行间隔</label><select id="schedule-interval" className="select" value={input.intervalMinutes} onChange={(event) => setInput({ ...input, intervalMinutes: Number(event.target.value) })}><option value={1}>每 1 分钟（测试）</option><option value={5}>每 5 分钟</option><option value={15}>每 15 分钟</option><option value={30}>每 30 分钟</option><option value={60}>每 1 小时</option><option value={360}>每 6 小时</option><option value={1440}>每 24 小时</option></select></div><div className="field"><label htmlFor="schedule-concurrency">并发数</label><select id="schedule-concurrency" className="select" value={input.concurrency} onChange={(event) => setInput({ ...input, concurrency: Number(event.target.value) })}>{[1, 2, 3, 5, 10].map((value) => <option key={value} value={value}>{value} 台并行</option>)}</select></div><div className="field"><label htmlFor="schedule-canary">灰度首批</label><select id="schedule-canary" className="select" value={input.canaryCount} onChange={(event) => setInput({ ...input, canaryCount: Number(event.target.value) })}>{[0, 1, 2, 3].map((value) => <option key={value} value={value}>{value === 0 ? "不灰度" : `${value} 台`}</option>)}</select></div></div><div className="field mt-4"><label htmlFor="schedule-timeout">单步超时（秒）</label><input id="schedule-timeout" className="input" type="number" min={10} max={86400} value={input.timeoutSeconds} onChange={(event) => setInput({ ...input, timeoutSeconds: Number(event.target.value) })} /><p className="field-help">每个步骤独立计时；超时后关闭 SSH 通道，远端进程是否退出需要人工核对。</p></div><div className="field mt-4"><span className="field-label">固定执行目标</span><div className="target-list schedule-targets">{servers.map((server) => <label className="target-item" key={server.id}><input type="checkbox" checked={input.serverIds.includes(server.id)} onChange={() => setInput({ ...input, serverIds: input.serverIds.includes(server.id) ? input.serverIds.filter((id) => id !== server.id) : [...input.serverIds, server.id] })} /><span><strong>{server.name}</strong><small>{server.username}@{server.host}:{server.port}</small></span></label>)}</div></div><label className="toggle-row mt-4"><input type="checkbox" checked={input.enabled} onChange={(event) => setInput({ ...input, enabled: event.target.checked })} /><span>启用计划；保存后等待一个完整间隔，不会立即执行</span></label></div>
      <div className="dialog-footer"><button className="btn" type="button" onClick={() => setEditorOpen(false)}>取消</button><button className="btn btn-primary" type="button" disabled={busy !== "" || !input.runbookId || input.serverIds.length === 0 || input.timeoutSeconds < 10} onClick={() => void save()}>保存计划</button></div>
    </dialog>
    <dialog ref={detailRef} className="dialog execution-detail-dialog" onCancel={(event) => { event.preventDefault(); if (!busy) setDetail(undefined); }}>
      <div className="dialog-header"><div><h2>{detail?.runbookName} · 执行详情</h2><span className="table-meta">{detail?.id} · 单步超时 {detail?.timeoutSeconds} 秒</span></div><button className="icon-btn" type="button" aria-label="关闭执行详情" onClick={() => setDetail(undefined)}><X size={18} /></button></div>
      <div className="dialog-body">{detail?.error && <div className="error-banner" role="alert">{detail.error}</div>}{detail?.status === "interrupted" && <div className="risk"><Clock3 size={18} /><p>App 退出使本地等待中断，不能据此判断远端命令是否成功。</p></div>}{detail?.result?.results.map((server) => <article className="panel batch-result" key={server.serverId}><div className="panel-header"><div className="batch-result-title">{server.success ? <CheckCircle2 size={19} /> : <XCircle size={19} />}<div><h3>{server.serverName}</h3><span className="table-meta">{server.durationMs} ms · 已执行 {server.steps.length} 步</span></div></div><span className={server.success ? "badge badge-ok" : "badge badge-bad"}>{server.success ? "成功" : "未完成"}</span></div>{server.error && <div className="error-banner m-4">{server.error}</div>}<div className="runbook-results">{server.steps.map((step, index) => <details key={step.stepId} open={!step.success}><summary><span>{index + 1}. {step.stepName}</span><span className={step.success ? "badge badge-ok" : "badge badge-bad"}>{step.outcome === "timed_out" ? "超时" : step.outcome === "cancelled" ? "取消" : step.exitCode === null ? "失败" : `退出码 ${step.exitCode}`}</span></summary>{step.error && <p className="runbook-step-error">{step.error}</p>}{step.output && <pre className="action-output"><code>{step.output}</code></pre>}<details className="batch-command"><summary>查看实际命令</summary><pre><code>{step.command}</code></pre></details></details>)}</div></article>)}{detail?.result && detail.result.skippedCount > 0 && <p className="field-help">{detail.result.skippedCount} 台目标因灰度失败或取消而未执行。</p>}</div>
      <div className="dialog-footer"><button className="btn" type="button" onClick={() => setDetail(undefined)}>关闭</button>{detail && active(detail) && <button className="btn btn-danger" type="button" disabled={busy !== ""} onClick={() => void cancel(detail)}><Square size={15} />取消任务</button>}{detail && !active(detail) && (detail.failureCount > 0 || detail.skippedCount > 0 || detail.error) && <button className="btn btn-primary" type="button" disabled={busy !== ""} onClick={() => void retry(detail)}><RotateCcw size={15} />重试失败目标</button>}</div>
    </dialog>
  </>;
}
