import { useCallback, useEffect, useRef, useState } from "react";
import { Activity, BellRing, CheckCircle2, Clock3, Globe2, Pause, Pencil, Play, Plus, RefreshCw, Settings, ShieldCheck, Trash2, X } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import { formatTime } from "../lib/format";
import type { ServiceEvent, ServiceMonitor, ServiceMonitorInput, ServiceMonitorView, ServiceSample, Server } from "../types";
import type { ServiceMonitoringController } from "../hooks/useServiceMonitoring";

const emptyDraft: ServiceMonitorInput = {
  name: "",
  kind: "http",
  target: "https://example.com",
  intervalSeconds: 60,
  timeoutSeconds: 10,
  expectedStatus: 200,
  keyword: "",
  failureThreshold: 3,
  certificateWarningDays: 30,
  enabled: true,
};

interface Props {
  servers: Server[];
  controller: ServiceMonitoringController;
  onSettings: () => void;
}

export function ServiceMonitorManager({ servers, controller, onSettings }: Props) {
  const { views, loading, checkingIds, errors, loadError, refresh, check } = controller;
  const [selectedId, setSelectedId] = useState("");
  const [hours, setHours] = useState(24);
  const [samples, setSamples] = useState<ServiceSample[]>([]);
  const [events, setEvents] = useState<ServiceEvent[]>([]);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [editor, setEditor] = useState<ServiceMonitorInput | null>(null);
  const [editingId, setEditingId] = useState<string>();
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const dialogRef = useRef<HTMLDialogElement>(null);
  const selected = views.find((view) => view.monitor.id === selectedId);
  const healthy = views.filter((view) => view.monitor.enabled && view.latest?.success && !hasActive(view, "availability")).length;
  const unavailable = views.filter((view) => hasActive(view, "availability")).length;
  const certificateWarnings = views.filter((view) => hasActive(view, "certificate")).length;

  useEffect(() => {
    if (!views.some((view) => view.monitor.id === selectedId)) setSelectedId(views[0]?.monitor.id ?? "");
  }, [views, selectedId]);

  useEffect(() => {
    if (editor && !dialogRef.current?.open) dialogRef.current?.showModal();
    if (!editor && dialogRef.current?.open) dialogRef.current.close();
  }, [editor]);

  useEffect(() => {
    if (!selectedId) {
      setSamples([]);
      setEvents([]);
      return;
    }
    let cancelled = false;
    setHistoryLoading(true);
    Promise.all([api.serviceSamples(selectedId, hours), api.serviceEvents(selectedId)])
      .then(([history, recentEvents]) => {
        if (cancelled) return;
        setSamples(history);
        setEvents(recentEvents);
      })
      .catch((reason) => { if (!cancelled) setError(errorMessage(reason)); })
      .finally(() => { if (!cancelled) setHistoryLoading(false); });
    return () => { cancelled = true; };
  }, [selectedId, hours, selected?.latest?.id]);

  const openEditor = (view?: ServiceMonitorView) => {
    setError("");
    setNotice("");
    setEditingId(view?.monitor.id);
    setEditor(view ? draftFrom(view.monitor) : { ...emptyDraft });
  };

  const save = async () => {
    if (!editor) return;
    setBusy("save");
    setError("");
    try {
      const saved = editingId
        ? await api.updateServiceMonitor(editingId, editor)
        : await api.createServiceMonitor(editor);
      await refresh();
      setSelectedId(saved.monitor.id);
      setEditor(null);
      setNotice(editingId ? "服务监控已更新" : "服务监控已创建，将立即开始第一次探测");
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy("");
    }
  };

  const toggle = async (view: ServiceMonitorView) => {
    setBusy(`toggle-${view.monitor.id}`);
    setError("");
    try {
      await api.setServiceMonitorEnabled(view.monitor.id, !view.monitor.enabled);
      await refresh();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy("");
    }
  };

  const remove = async (view: ServiceMonitorView) => {
    if (!window.confirm(`确认删除“${view.monitor.name}”？它的探测历史和事件也会删除。`)) return;
    setBusy(`delete-${view.monitor.id}`);
    setError("");
    try {
      await api.deleteServiceMonitor(view.monitor.id);
      await refresh();
      setNotice("服务监控及其历史已删除");
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy("");
    }
  };

  const checkNow = async (view: ServiceMonitorView) => {
    setError("");
    setNotice("");
    const result = await check(view.monitor.id);
    if (result) {
      setNotice(`${view.monitor.name}：${result.view.latest?.detail ?? "检查完成"}`);
      if (result.notificationError) setError(result.notificationError);
    }
  };

  const update = <K extends keyof ServiceMonitorInput>(key: K, value: ServiceMonitorInput[K]) => {
    setEditor((current) => current ? { ...current, [key]: value } : current);
  };

  return <>
    <div className="stats monitoring-stats service-stats">
      <div className="stat"><span className="stat-label">监控服务</span><strong className="stat-value">{views.length}</strong><span className="stat-note">{views.filter((view) => !view.monitor.enabled).length} 个已暂停</span></div>
      <div className="stat"><span className="stat-label">当前正常</span><strong className="stat-value">{healthy}</strong><span className="stat-note">最近一次探测成功</span></div>
      <div className="stat"><span className="stat-label">服务故障</span><strong className={`stat-value ${unavailable ? "text-bad" : ""}`}>{unavailable}</strong><span className="stat-note">达到连续失败阈值</span></div>
      <div className="stat"><span className="stat-label">证书告警</span><strong className={`stat-value ${certificateWarnings ? "text-bad" : ""}`}>{certificateWarnings}</strong><span className="stat-note">临期或已经过期</span></div>
    </div>
    <div className="service-local-notice"><Globe2 size={18} /><p>探测从这台 Mac 发起，支持 HTTP/HTTPS、TCP 和 DNS。应用退出后暂停；关联 VPS 只用于资产归类，不经过 SSH。</p><div className="heading-actions"><button className="btn" type="button" disabled={loading || busy !== ""} onClick={() => void refresh()}><RefreshCw size={15} />刷新</button><button className="btn btn-primary" type="button" onClick={() => openEditor()}><Plus size={16} />添加监控</button></div></div>
    {(error || loadError) && <div className="error-banner" role="alert">{error || loadError}</div>}
    {notice && <div className="file-notice service-notice" role="status">{notice}</div>}
    {Object.entries(errors).filter(([, message]) => message).map(([id, message]) => <div className="error-banner" role="alert" key={id}>{views.find((view) => view.monitor.id === id)?.monitor.name ?? "服务监控"}：{message}</div>)}
    <WebhookSettings onSettings={onSettings} />
    {loading && views.length === 0 ? <div className="skeleton service-table-skeleton" /> : views.length === 0 ? <div className="empty"><div><Globe2 size={31} /><h2>添加第一个服务监控</h2><p>监控网站、API、数据库端口或 DNS，并记录可用率、延迟和故障恢复事件。</p><button className="btn btn-primary" type="button" onClick={() => openEditor()}><Plus size={16} />添加监控</button></div></div> : <section className="panel service-monitor-panel">
      <div className="panel-header"><div><h2>服务状态</h2><span className="table-meta">{checkingIds.length ? `正在探测 ${checkingIds.length} 个服务` : "按各自间隔自动探测，最多 4 个并行"}</span></div></div>
      <div className="table-wrap"><table className="data-table service-monitor-table"><thead><tr><th>服务</th><th>状态</th><th>响应</th><th>可用率 24h / 7d / 30d</th><th>TLS 证书</th><th>最近检查</th><th className="text-right">操作</th></tr></thead><tbody>{views.map((view) => <tr key={view.monitor.id} className={selectedId === view.monitor.id ? "service-row is-selected" : "service-row"} onClick={() => setSelectedId(view.monitor.id)}>
        <td><span className="table-name">{view.monitor.name}</span><code className="table-meta service-target" title={view.monitor.target}>{targetLabel(view.monitor)}</code><span className="table-meta">{kindLabel(view.monitor.kind)} · 每 {formatInterval(view.monitor.intervalSeconds)}{serverName(view.monitor, servers) ? ` · ${serverName(view.monitor, servers)}` : ""}</span></td>
        <td><MonitorStatus view={view} />{!view.monitor.enabled && <span className="table-meta">不会自动探测</span>}</td>
        <td className="measured">{view.latest ? `${view.latest.latencyMs} ms` : "—"}{view.latest?.statusCode && <span className="table-meta">HTTP {view.latest.statusCode}</span>}</td>
        <td className="measured service-uptime">{formatUptime(view.uptime24h)} / {formatUptime(view.uptime7d)} / {formatUptime(view.uptime30d)}</td>
        <td>{certificateLabel(view)}</td>
        <td className="log-time">{view.latest ? formatTime(view.latest.checkedAt) : "等待首次探测"}</td>
        <td><div className="row-actions"><button className="icon-btn" type="button" title="立即探测" aria-label={`立即探测 ${view.monitor.name}`} disabled={checkingIds.includes(view.monitor.id) || busy !== ""} onClick={(event) => { event.stopPropagation(); void checkNow(view); }}><RefreshCw size={15} /></button><button className="icon-btn" type="button" title={view.monitor.enabled ? "暂停自动探测" : "恢复自动探测"} aria-label={`${view.monitor.enabled ? "暂停" : "恢复"} ${view.monitor.name}`} disabled={busy !== ""} onClick={(event) => { event.stopPropagation(); void toggle(view); }}>{view.monitor.enabled ? <Pause size={15} /> : <Play size={15} />}</button><button className="icon-btn" type="button" title="编辑" aria-label={`编辑 ${view.monitor.name}`} disabled={busy !== ""} onClick={(event) => { event.stopPropagation(); openEditor(view); }}><Pencil size={15} /></button><button className="icon-btn" type="button" title="删除" aria-label={`删除 ${view.monitor.name}`} disabled={busy !== ""} onClick={(event) => { event.stopPropagation(); void remove(view); }}><Trash2 size={15} /></button></div></td>
      </tr>)}</tbody></table></div>
    </section>}
    {selected && <section className="panel service-history">
      <div className="panel-header"><div><h2>{selected.monitor.name} · 延迟与事件</h2><span className="table-meta">本地保存 30 天，最多保留 720 个展示采样点</span></div><select className="select" aria-label="服务历史时间范围" value={hours} onChange={(event) => setHours(Number(event.target.value))}><option value={1}>最近 1 小时</option><option value={24}>最近 24 小时</option><option value={168}>最近 7 天</option><option value={720}>最近 30 天</option></select></div>
      {selected.latest && <div className={`service-latest ${selected.latest.success ? "is-success" : "is-failed"}`}>{selected.latest.success ? <CheckCircle2 size={18} /> : <BellRing size={18} />}<div><strong>{selected.latest.success ? "最近探测成功" : `最近探测失败 · 连续 ${selected.consecutiveFailures} 次`}</strong><p>{selected.latest.detail}</p></div></div>}
      {historyLoading && samples.length === 0 ? <div className="skeleton chart-skeleton" /> : samples.length === 0 ? <div className="empty chart-empty"><div><Activity size={27} /><h2>还没有服务采样</h2><p>点击表格中的立即探测，或等待下一个自动探测周期。</p></div></div> : <ServiceLatencyChart samples={samples} />}
      <div className="service-event-list"><div className="panel-header"><div><h3>故障与恢复事件</h3><span className="table-meta">可用性和证书分别跟踪</span></div></div>{events.length === 0 ? <div className="alert-empty"><Clock3 size={23} /><p>暂无事件</p></div> : <div className="table-wrap"><table className="data-table"><thead><tr><th>状态</th><th>类型</th><th>事件</th><th>时间</th></tr></thead><tbody>{events.slice(0, 20).map((event) => <tr key={event.id}><td><span className={event.status === "triggered" ? "badge badge-bad" : "badge badge-ok"}>{event.status === "triggered" ? "触发" : "恢复"}</span></td><td>{event.kind === "availability" ? "可用性" : "TLS 证书"}</td><td>{event.message}</td><td className="log-time">{formatTime(event.createdAt)}</td></tr>)}</tbody></table></div>}</div>
    </section>}
    <dialog ref={dialogRef} className="dialog service-monitor-dialog" onCancel={(event) => { event.preventDefault(); if (!busy) setEditor(null); }}>
      <form onSubmit={(event) => { event.preventDefault(); void save(); }}>
        <div className="dialog-header"><h2>{editingId ? "编辑服务监控" : "添加服务监控"}</h2><button className="icon-btn" type="button" aria-label="关闭" disabled={busy !== ""} onClick={() => setEditor(null)}><X size={18} /></button></div>
        {editor && <div className="dialog-body">
          {error && <div className="error-banner" role="alert">{error}</div>}
          <div className="form-grid"><div className="field"><label htmlFor="service-name">服务名称</label><input id="service-name" className="input" required maxLength={120} value={editor.name} onChange={(event) => update("name", event.target.value)} placeholder="例如：官网健康检查" /></div><div className="field"><label htmlFor="service-kind">探测类型</label><select id="service-kind" className="select" value={editor.kind} onChange={(event) => { const kind = event.target.value as ServiceMonitorInput["kind"]; setEditor((current) => current ? { ...current, kind, port: kind === "tcp" ? current.port ?? 443 : current.port, target: current.target === emptyDraft.target && kind !== "http" ? "" : current.target } : current); }}><option value="http">HTTP / HTTPS</option><option value="tcp">TCP 端口</option><option value="dns">DNS 解析</option></select></div>
            <div className={`field ${editor.kind === "tcp" ? "" : "span-2"}`}><label htmlFor="service-target">{editor.kind === "http" ? "完整 URL" : editor.kind === "tcp" ? "主机名或 IP" : "要解析的域名"}</label><input id="service-target" className="input" required spellCheck={false} value={editor.target} onChange={(event) => update("target", event.target.value)} placeholder={editor.kind === "http" ? "https://example.com/health" : editor.kind === "tcp" ? "db.example.com" : "example.com"} /><p className="field-help">{editor.kind === "http" ? "HTTPS 会同时读取证书到期时间；URL 不允许包含明文密码。" : editor.kind === "tcp" ? "探测从本机发起，不经过关联 VPS。" : "返回至少一个 IPv4 或 IPv6 地址即视为解析成功。"}</p></div>
            {editor.kind === "tcp" && <div className="field"><label htmlFor="service-port">TCP 端口</label><input id="service-port" className="input" type="number" min={1} max={65535} required value={editor.port ?? ""} onChange={(event) => update("port", Number(event.target.value))} /></div>}
            <div className="field"><label htmlFor="service-server">关联 VPS（可选）</label><select id="service-server" className="select" value={editor.serverId ?? ""} onChange={(event) => update("serverId", event.target.value || undefined)}><option value="">不关联</option>{servers.map((server) => <option value={server.id} key={server.id}>{server.name}</option>)}</select></div><div className="field"><label htmlFor="service-interval">探测间隔</label><select id="service-interval" className="select" value={editor.intervalSeconds} onChange={(event) => update("intervalSeconds", Number(event.target.value))}><option value={15}>每 15 秒</option><option value={30}>每 30 秒</option><option value={60}>每 60 秒</option><option value={300}>每 5 分钟</option><option value={900}>每 15 分钟</option><option value={3600}>每 1 小时</option></select></div>
            <div className="field"><label htmlFor="service-timeout">超时时间（秒）</label><input id="service-timeout" className="input" type="number" min={1} max={30} value={editor.timeoutSeconds} onChange={(event) => update("timeoutSeconds", Number(event.target.value))} /></div><div className="field"><label htmlFor="service-failures">连续失败判定</label><select id="service-failures" className="select" value={editor.failureThreshold} onChange={(event) => update("failureThreshold", Number(event.target.value))}><option value={1}>连续 1 次失败</option><option value={2}>连续 2 次失败</option><option value={3}>连续 3 次失败</option><option value={5}>连续 5 次失败</option><option value={10}>连续 10 次失败</option></select></div>
            {editor.kind === "http" && <><div className="field"><label htmlFor="service-status">期望 HTTP 状态码</label><input id="service-status" className="input" type="number" min={100} max={599} value={editor.expectedStatus} onChange={(event) => update("expectedStatus", Number(event.target.value))} /></div><div className="field"><label htmlFor="service-cert-days">证书提前预警（天）</label><input id="service-cert-days" className="input" type="number" min={1} max={365} value={editor.certificateWarningDays} onChange={(event) => update("certificateWarningDays", Number(event.target.value))} /></div><div className="field span-2"><label htmlFor="service-keyword">响应关键字（可选）</label><input id="service-keyword" className="input" spellCheck={false} value={editor.keyword ?? ""} onChange={(event) => update("keyword", event.target.value)} placeholder="例如：ok；只检查响应前 256 KiB" /></div></>}
          </div>
          <label className="backup-credential-option"><input type="checkbox" checked={editor.enabled} onChange={(event) => update("enabled", event.target.checked)} /><span><strong>启用自动探测</strong><small>暂停后仍可手动检查；已有历史不会删除。</small></span></label>
        </div>}
        <div className="dialog-footer"><button className="btn" type="button" disabled={busy !== ""} onClick={() => setEditor(null)}>取消</button><button className="btn btn-primary" type="submit" disabled={busy !== "" || !editor?.name.trim() || !editor.target.trim()}>{busy === "save" ? "正在保存" : "保存监控"}</button></div>
      </form>
    </dialog>
  </>;
}

function WebhookSettings({ onSettings }: { onSettings: () => void }) {
  const [settings, setSettings] = useState<{ enabled: boolean; hasWebhookUrl: boolean }>();
  const [enabled, setEnabled] = useState(false);
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");

  const load = useCallback(async () => {
    try {
      const saved = await api.serviceNotificationSettings();
      setSettings(saved);
      setEnabled(saved.enabled);
    } catch (reason) { setError(errorMessage(reason)); }
  }, []);
  useEffect(() => { void load(); }, [load]);

  const save = async (clearUrl = false) => {
    setBusy("save");
    setError("");
    setNotice("");
    try {
      const saved = await api.updateServiceNotificationSettings(clearUrl ? false : enabled, clearUrl ? undefined : url.trim() || undefined, clearUrl);
      setSettings(saved);
      setEnabled(saved.enabled);
      setUrl("");
      setNotice(clearUrl ? "Webhook URL 已从 Keychain 删除" : "Webhook 设置已保存");
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };

  const test = async () => {
    setBusy("test");
    setError("");
    setNotice("");
    try { await api.testServiceWebhook(); setNotice("测试事件已发送，接收端返回成功"); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };

  return <section className="panel webhook-panel"><div className="panel-header"><div><h2>服务事件通知</h2><span className="table-meta">故障、恢复和证书事件发送 JSON Webhook；URL 仅保存在 macOS Keychain</span></div><ShieldCheck size={20} /></div><div className="panel-body webhook-body">
    {error && <div className="error-banner" role="alert">{error}</div>}{notice && <div className="file-notice" role="status">{notice}</div>}
    <div className="webhook-form"><label className="switch-control"><input type="checkbox" checked={enabled} onChange={(event) => setEnabled(event.target.checked)} /><span>{enabled ? "Webhook 已启用" : "Webhook 已关闭"}</span></label><input className="input" type="password" aria-label="Webhook URL" autoComplete="off" spellCheck={false} value={url} onChange={(event) => setUrl(event.target.value)} placeholder={settings?.hasWebhookUrl ? "已保存到 Keychain；留空保持不变" : "https://hooks.example.com/controlvps"} /><button className="btn" type="button" disabled={busy !== "" || (enabled && !settings?.hasWebhookUrl && !url.trim())} onClick={() => void save()}>{busy === "save" ? "正在保存" : "保存"}</button><button className="btn" type="button" disabled={busy !== "" || !settings?.hasWebhookUrl} onClick={() => void test()}>{busy === "test" ? "正在发送" : "发送测试"}</button>{settings?.hasWebhookUrl && <button className="btn btn-danger" type="button" disabled={busy !== ""} onClick={() => void save(true)}>清除 URL</button>}</div>
    <div className="webhook-foot"><span>系统通知沿用全局告警设置。</span><button className="table-link" type="button" onClick={onSettings}><Settings size={14} />调整系统通知</button></div>
  </div></section>;
}

function MonitorStatus({ view }: { view: ServiceMonitorView }) {
  if (!view.monitor.enabled) return <span className="badge">已暂停</span>;
  if (hasActive(view, "availability")) return <span className="badge badge-bad">故障</span>;
  if (view.latest && !view.latest.success) return <span className="badge badge-bad">探测失败</span>;
  if (hasActive(view, "certificate")) return <span className="badge badge-cool">证书临期</span>;
  if (view.latest?.success) return <span className="badge badge-ok">正常</span>;
  return <span className="badge">等待探测</span>;
}

function ServiceLatencyChart({ samples }: { samples: ServiceSample[] }) {
  const width = 760;
  const height = 190;
  const top = 18;
  const bottom = 150;
  const maximum = Math.max(1, ...samples.map((sample) => sample.latencyMs)) * 1.1;
  const x = (index: number) => samples.length === 1 ? width / 2 : index / (samples.length - 1) * width;
  const y = (value: number) => bottom - Math.max(0, Math.min(maximum, value)) / maximum * (bottom - top);
  const points = samples.map((sample, index) => `${x(index).toFixed(2)},${y(sample.latencyMs).toFixed(2)}`).join(" ");
  const latest = samples[samples.length - 1];
  return <div className="service-latency-chart"><div className="trend-heading"><div><h3>响应时间</h3><span className="table-meta">毫秒 · 红色点表示失败采样</span></div><div className="trend-legend"><span><i style={{ background: "var(--color-cool)" }} />最新 <strong>{latest.latencyMs} ms</strong></span></div></div><svg viewBox={`0 0 ${width} ${height}`} role="img" aria-label={`服务响应时间趋势，共 ${samples.length} 个采样点`} preserveAspectRatio="none">{[0, 0.5, 1].map((ratio) => <line key={ratio} x1="0" x2={width} y1={bottom - ratio * (bottom - top)} y2={bottom - ratio * (bottom - top)} className="chart-rule" />)}<polyline points={points} fill="none" stroke="var(--color-cool)" strokeWidth="2" vectorEffect="non-scaling-stroke" />{samples.map((sample, index) => !sample.success && <circle key={sample.id} cx={x(index)} cy={y(sample.latencyMs)} r="4" fill="var(--color-bad)" vectorEffect="non-scaling-stroke" />)}</svg><div className="chart-times"><span>{formatTime(samples[0].checkedAt)}</span><span>{formatTime(latest.checkedAt)}</span></div></div>;
}

function draftFrom(monitor: ServiceMonitor): ServiceMonitorInput {
  return {
    name: monitor.name,
    kind: monitor.kind,
    target: monitor.target,
    port: monitor.port,
    serverId: monitor.serverId,
    intervalSeconds: monitor.intervalSeconds,
    timeoutSeconds: monitor.timeoutSeconds,
    expectedStatus: monitor.expectedStatus,
    keyword: monitor.keyword ?? "",
    failureThreshold: monitor.failureThreshold,
    certificateWarningDays: monitor.certificateWarningDays,
    enabled: monitor.enabled,
  };
}

function hasActive(view: ServiceMonitorView, kind: ServiceEvent["kind"]) {
  return view.activeEvents.some((event) => event.kind === kind);
}

function targetLabel(monitor: ServiceMonitor) {
  return monitor.kind === "tcp" ? `${monitor.target}:${monitor.port ?? "—"}` : monitor.target;
}

function kindLabel(kind: ServiceMonitor["kind"]) {
  return { http: "HTTP", tcp: "TCP", dns: "DNS" }[kind];
}

function serverName(monitor: ServiceMonitor, servers: Server[]) {
  return servers.find((server) => server.id === monitor.serverId)?.name;
}

function formatUptime(value?: number) {
  return value === undefined ? "—" : `${value.toFixed(value >= 99.95 ? 1 : 2)}%`;
}

function formatInterval(seconds: number) {
  if (seconds < 60) return `${seconds} 秒`;
  if (seconds < 3600) return `${seconds / 60} 分钟`;
  return `${seconds / 3600} 小时`;
}

function certificateLabel(view: ServiceMonitorView) {
  const days = view.latest?.certificateDaysRemaining;
  if (days === undefined) return <span className="table-meta">不适用</span>;
  const expires = view.latest?.certificateExpiresAt;
  if (days < 0) return <span className="badge badge-bad" title={expires}>已过期 {Math.abs(days)} 天</span>;
  if (days <= view.monitor.certificateWarningDays) return <span className="badge badge-bad" title={expires}>剩余 {days} 天</span>;
  return <span className="table-meta" title={expires}>剩余 {days} 天</span>;
}
