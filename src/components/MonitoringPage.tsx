import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Activity, BellRing, Globe2, RefreshCw, Settings, TriangleAlert } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import { formatBytes, formatTime, percent } from "../lib/format";
import type { AlertEvent, MetricSample, MonitoringSettings, Server, ServerMetrics, TrustedExecutor } from "../types";
import type { ServiceMonitoringController } from "../hooks/useServiceMonitoring";
import { ServiceMonitorManager } from "./ServiceMonitorManager";

interface Props {
  servers: Server[];
  metrics: Record<string, ServerMetrics>;
  errors: Record<string, string>;
  settings?: MonitoringSettings;
  trusted: TrustedExecutor;
  onMetrics: (serverId: string, metrics: ServerMetrics) => void;
  onSettings: () => void;
  serviceMonitoring: ServiceMonitoringController;
}

type Series = {
  name: string;
  color: string;
  value: (sample: MetricSample) => number;
};

export function MonitoringPage(props: Props) {
  const [mode, setMode] = useState<"resources" | "services">("resources");
  return <>
    <div className="page-heading"><div><h1>监控与告警</h1><p>同时跟踪 VPS 主机资源和网站、端口、DNS 的实际可用性。</p></div></div>
    <div className="workspace-switch" role="tablist" aria-label="监控类型"><button className="tab" type="button" role="tab" aria-selected={mode === "resources"} onClick={() => setMode("resources")}><Activity size={16} />主机资源</button><button className="tab" type="button" role="tab" aria-selected={mode === "services"} onClick={() => setMode("services")}><Globe2 size={16} />服务可用性</button></div>
    {mode === "resources" ? <ResourceMonitoring {...props} /> : <ServiceMonitorManager servers={props.servers} controller={props.serviceMonitoring} onSettings={props.onSettings} />}
  </>;
}

function ResourceMonitoring({ servers, metrics, errors, settings, trusted, onMetrics, onSettings }: Props) {
  const [selectedId, setSelectedId] = useState(servers[0]?.id ?? "");
  const [hours, setHours] = useState(24);
  const [samples, setSamples] = useState<MetricSample[]>([]);
  const [events, setEvents] = useState<AlertEvent[]>([]);
  const [activeAlerts, setActiveAlerts] = useState<AlertEvent[]>([]);
  const [loading, setLoading] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState("");
  const selected = servers.find((server) => server.id === selectedId);
  const reading = selected ? metrics[selected.id] : undefined;

  useEffect(() => {
    if (!servers.some((server) => server.id === selectedId)) setSelectedId(servers[0]?.id ?? "");
  }, [servers, selectedId]);

  const loadAlerts = useCallback(async () => {
    try {
      const [recent, active] = await Promise.all([api.alertEvents(), api.activeAlerts()]);
      setEvents(recent);
      setActiveAlerts(active);
    } catch (reason) {
      setError(errorMessage(reason));
    }
  }, []);

  useEffect(() => {
    void loadAlerts();
    let unlisten: (() => void) | undefined;
    void listen<AlertEvent>("monitoring-alert", () => { void loadAlerts(); }).then((dispose) => { unlisten = dispose; });
    return () => unlisten?.();
  }, [loadAlerts]);

  useEffect(() => {
    if (!selectedId) {
      setSamples([]);
      return;
    }
    let cancelled = false;
    setLoading(true);
    api.metricHistory(selectedId, hours)
      .then((history) => { if (!cancelled) setSamples(history); })
      .catch((reason) => { if (!cancelled) setError(errorMessage(reason)); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [selectedId, hours, reading?.collectedAt]);

  const refresh = async () => {
    if (!selected) return;
    setRefreshing(true);
    setError("");
    try {
      const result = await trusted(selected, (allow) => api.collectMetrics(selected.id, allow));
      onMetrics(selected.id, result);
      await loadAlerts();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setRefreshing(false);
    }
  };

  const selectedAlerts = useMemo(() => activeAlerts.filter((event) => event.serverId === selectedId), [activeAlerts, selectedId]);

  return <>
    <div className="section-heading automation-subheading"><span className="table-meta">通过 SSH 读取 Linux 指标，本地保存 7 天历史采样</span><div className="heading-actions">
        <select className="select" aria-label="监控服务器" value={selectedId} onChange={(event) => setSelectedId(event.target.value)}>
          {servers.map((server) => <option value={server.id} key={server.id}>{server.name}</option>)}
        </select>
        <button className="btn btn-primary" type="button" onClick={refresh} disabled={!selected || refreshing}><RefreshCw size={16} />{refreshing ? "正在采集" : "立即采集"}</button>
      </div>
    </div>
    {error && <div className="error-banner" role="alert"><p>{error}</p><button className="btn" type="button" onClick={() => { setError(""); void loadAlerts(); }}>重试</button></div>}
    <div className="stats monitoring-stats">
      <div className="stat"><span className="stat-label">活动告警</span><strong className="stat-value">{activeAlerts.length}</strong><span className="stat-note">全部 VPS</span></div>
      <div className="stat"><span className="stat-label">当前服务器</span><strong className="stat-value text-base">{selected?.name ?? "未选择"}</strong><span className="stat-note">{selectedAlerts.length ? `${selectedAlerts.length} 条活动告警` : "无活动告警"}</span></div>
      <div className="stat"><span className="stat-label">自动采集</span><strong className="stat-value text-base">{settings?.autoCollect ? `每 ${settings.intervalSeconds} 秒` : "已关闭"}</strong><span className="stat-note">仅在应用运行时执行</span></div>
      <div className="stat"><span className="stat-label">系统通知</span><strong className="stat-value text-base">{settings?.notificationsEnabled ? "已启用" : "已关闭"}</strong><button className="table-link table-meta" type="button" onClick={onSettings}>调整告警规则</button></div>
    </div>
    {!selected ? <div className="empty"><div><Activity size={32} /><h2>添加 VPS 后开始监控</h2><p>通过 SSH 采集，无需在服务器安装 Agent。</p></div></div> : <>
      {selectedAlerts.length > 0 && <div className="alert-summary" role="alert"><TriangleAlert size={20} /><div><strong>{selected.name} 有 {selectedAlerts.length} 条活动告警</strong>{selectedAlerts.map((event) => <p key={event.id}>{event.message}</p>)}</div></div>}
      {errors[selected.id] && <div className="error-banner" role="alert">最近一次自动采集失败：{errors[selected.id]}</div>}
      <section className="panel">
        <div className="panel-header"><div><h2>资源趋势</h2><span className="table-meta">{samples.length} 个采样点 · 数据仅保存在本机</span></div><select className="select" aria-label="历史时间范围" value={hours} onChange={(event) => setHours(Number(event.target.value))}><option value={1}>最近 1 小时</option><option value={6}>最近 6 小时</option><option value={24}>最近 24 小时</option><option value={168}>最近 7 天</option></select></div>
        {loading && samples.length === 0 ? <div className="skeleton chart-skeleton" /> : samples.length === 0 ? <div className="empty chart-empty"><div><Activity size={28} /><h2>这个时间范围还没有采样</h2><p>点击“立即采集”，或在设置中开启全部 VPS 自动采集。</p><button className="btn btn-primary" type="button" onClick={refresh} disabled={refreshing}>立即采集</button></div></div> : <div className="chart-grid">
          <TrendChart title="CPU 使用率" unit="百分比" samples={samples} series={[{ name: "CPU", color: "var(--color-accent)", value: (sample) => sample.cpuUsage }]} fixedMax={100} formatter={(value) => `${value.toFixed(1)}%`} />
          <TrendChart title="内存使用率" unit="百分比" samples={samples} series={[{ name: "内存", color: "var(--color-cool)", value: (sample) => sample.memoryUsage }]} fixedMax={100} formatter={(value) => `${value.toFixed(1)}%`} />
          <TrendChart title="系统盘使用率" unit="百分比" samples={samples} series={[{ name: "系统盘", color: "var(--color-bad)", value: (sample) => sample.diskUsage }]} fixedMax={100} formatter={(value) => `${value.toFixed(1)}%`} />
          <TrendChart title="网络吞吐" unit="字节每秒" samples={samples} series={[{ name: "下行", color: "var(--color-ok)", value: (sample) => sample.networkRxBytesPerSecond }, { name: "上行", color: "var(--color-cool)", value: (sample) => sample.networkTxBytesPerSecond }]} formatter={(value) => formatBytes(value, true)} />
        </div>}
      </section>
      {reading && <div className="monitoring-current"><span>最新：{formatTime(reading.collectedAt)}</span><span>CPU {reading.cpuUsage.toFixed(1)}%</span><span>内存 {percent(reading.memoryUsed, reading.memoryTotal).toFixed(1)}%</span><span>负载 {reading.loadAverage.map((value) => value.toFixed(2)).join(" / ")}</span></div>}
    </>}
    <section className="panel alert-history">
      <div className="panel-header"><div><h2>告警事件</h2><span className="stat-note">阈值触发、恢复与连续采集失败都会留痕</span></div><button className="btn" type="button" onClick={onSettings}><Settings size={15} />告警设置</button></div>
      {events.length === 0 ? <div className="alert-empty"><BellRing size={24} /><p>暂无告警事件</p></div> : <div className="table-wrap"><table className="data-table"><thead><tr><th>状态</th><th>服务器</th><th>事件</th><th>时间</th></tr></thead><tbody>{events.slice(0, 20).map((event) => <tr key={event.id}><td><span className={event.status === "triggered" ? "badge badge-bad" : "badge badge-ok"}>{event.status === "triggered" ? "触发" : "恢复"}</span></td><td><span className="table-name">{event.serverName}</span><span className="table-meta">{alertKind(event)}</span></td><td>{event.message}</td><td className="log-time">{formatTime(event.createdAt)}</td></tr>)}</tbody></table></div>}
    </section>
  </>;
}

function TrendChart({ title, unit, samples, series, fixedMax, formatter }: { title: string; unit: string; samples: MetricSample[]; series: Series[]; fixedMax?: number; formatter: (value: number) => string }) {
  const width = 640;
  const height = 170;
  const top = 16;
  const bottom = 142;
  const maximum = fixedMax ?? Math.max(1, ...samples.flatMap((sample) => series.map((item) => item.value(sample)))) * 1.1;
  const x = (index: number) => samples.length === 1 ? width / 2 : index / (samples.length - 1) * width;
  const y = (value: number) => bottom - Math.max(0, Math.min(maximum, value)) / maximum * (bottom - top);
  return <article className="trend-chart">
    <div className="trend-heading"><div><h3>{title}</h3><span className="table-meta">{unit}</span></div><div className="trend-legend">{series.map((item) => <span key={item.name}><i style={{ background: item.color }} />{item.name} <strong>{formatter(item.value(samples[samples.length - 1]))}</strong></span>)}</div></div>
    <svg viewBox={`0 0 ${width} ${height}`} role="img" aria-label={`${title}趋势，共 ${samples.length} 个采样点`} preserveAspectRatio="none">
      {[0, 0.5, 1].map((ratio) => <line key={ratio} x1="0" x2={width} y1={bottom - ratio * (bottom - top)} y2={bottom - ratio * (bottom - top)} className="chart-rule" />)}
      {series.map((item) => {
        const points = samples.map((sample, index) => `${x(index).toFixed(2)},${y(item.value(sample)).toFixed(2)}`).join(" ");
        const last = item.value(samples[samples.length - 1]);
        return <g key={item.name}><polyline points={points} fill="none" stroke={item.color} strokeWidth="2" vectorEffect="non-scaling-stroke" /><circle cx={x(samples.length - 1)} cy={y(last)} r="3" fill={item.color} /></g>;
      })}
    </svg>
    <div className="chart-times"><span>{formatTime(samples[0].collectedAt)}</span><span>{formatTime(samples[samples.length - 1].collectedAt)}</span></div>
  </article>;
}

function alertKind(event: AlertEvent) {
  return { offline: "连接", cpu: "CPU", memory: "内存", disk: "磁盘" }[event.kind];
}
