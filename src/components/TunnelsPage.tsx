import { useCallback, useEffect, useState } from "react";
import { ExternalLink, Network, Play, Plus, RefreshCw, Server as ServerIcon, Square, Trash2 } from "lucide-react";
import { api, errorMessage, openExternal } from "../lib/api";
import { formatTime } from "../lib/format";
import type { PortForwardInput, PortForwardRule, Server, TrustedExecutor, TunnelStatus } from "../types";

const initialForm: PortForwardInput = {
  name: "",
  serverId: "",
  kind: "local",
  listenAddress: "127.0.0.1",
  listenPort: 8080,
  destinationHost: "127.0.0.1",
  destinationPort: 80,
};

interface Props {
  servers: Server[];
  trusted: TrustedExecutor;
  onAdd: () => void;
}

export function TunnelsPage({ servers, trusted, onAdd }: Props) {
  const [rules, setRules] = useState<PortForwardRule[]>([]);
  const [statuses, setStatuses] = useState<TunnelStatus[]>([]);
  const [form, setForm] = useState<PortForwardInput>(initialForm);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");

  const refreshStatuses = useCallback(async () => {
    try { setStatuses(await api.listTunnelStatuses()); }
    catch (reason) { setError(errorMessage(reason)); }
  }, []);
  const load = useCallback(async () => {
    setError("");
    try {
      const [nextRules, nextStatuses] = await Promise.all([api.listPortForwards(), api.listTunnelStatuses()]);
      setRules(nextRules);
      setStatuses(nextStatuses);
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setLoading(false); }
  }, []);
  useEffect(() => { void load(); }, [load]);
  useEffect(() => {
    if (!form.serverId && servers[0]) setForm((current) => ({ ...current, serverId: servers[0].id }));
  }, [servers, form.serverId]);
  useEffect(() => {
    if (statuses.length === 0) return;
    const timer = window.setInterval(() => void refreshStatuses(), 2000);
    return () => window.clearInterval(timer);
  }, [statuses.length, refreshStatuses]);

  const save = async () => {
    setBusy("create");
    setError("");
    setNotice("");
    try {
      await api.createPortForward(form);
      setForm({ ...initialForm, serverId: form.serverId });
      await load();
      setNotice(`${form.kind === "dynamic" ? "SOCKS5 动态转发" : form.kind === "remote" ? "远程端口转发" : "本地端口转发"}规则已保存，启动后才会开始监听`);
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };
  const start = async (rule: PortForwardRule) => {
    const server = servers.find((item) => item.id === rule.serverId);
    if (!server) return setError("规则关联的 VPS 已不存在，请删除后重新创建");
    setBusy(rule.id);
    setError("");
    setNotice("");
    try {
      const status = await trusted(server, (allow) => api.startPortForward(rule.id, allow));
      setNotice(`${rule.name} 已监听 ${status.localEndpoint}${rule.kind === "dynamic" ? "，客户端代理类型请选择 SOCKS5" : rule.kind === "remote" ? "，连接将转发到本机目标服务" : ""}`);
      await refreshStatuses();
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };
  const stop = async (rule: PortForwardRule) => {
    setBusy(rule.id);
    setError("");
    try {
      await api.stopPortForward(rule.id);
      setNotice(`${rule.name} 已停止`);
      await refreshStatuses();
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };
  const remove = async (rule: PortForwardRule, running: boolean) => {
    if (!window.confirm(`确认删除规则 ${rule.name}？${running ? "活动隧道会同时停止。" : ""}`)) return;
    setBusy(rule.id);
    setError("");
    try {
      if (running) await api.stopPortForward(rule.id);
      await api.deletePortForward(rule.id);
      await load();
      setNotice("转发规则已删除");
    } catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };

  const statusMap = new Map(statuses.map((status) => [status.ruleId, status]));
  return <>
    <div className="page-heading"><div><h1>隧道中心</h1><p>通过 SSH 使用本地、远程端口转发或动态 SOCKS5 代理。</p></div><button className="btn" type="button" onClick={() => void load()} disabled={loading || busy !== ""}><RefreshCw size={16} />刷新状态</button></div>
    <div className="risk tunnel-note"><Network size={20} /><p>本地转发将 VPS 侧服务映射到本机；远程转发将本机服务映射到 VPS；动态转发提供 SOCKS5 代理。监听端口填 0 可自动分配，非本机监听需 SSH 服务允许 GatewayPorts。</p></div>
    {error && <div className="error-banner" role="alert">{error}</div>}
    {notice && <div className="file-notice automation-notice" role="status">{notice}</div>}
    <section className="panel tunnel-form-panel">
      <div className="panel-header"><div><h2>新建转发规则</h2><span className="table-meta">规则持久化保存，应用重启后不会自动启动</span></div><Network size={19} /></div>
      {servers.length === 0 ? <div className="empty"><div><ServerIcon size={28} /><h2>先添加 VPS</h2><p>端口转发需要一台已配置 SSH 的服务器。</p><button className="btn btn-primary" type="button" onClick={onAdd}>添加 VPS</button></div></div> : <form onSubmit={(event) => { event.preventDefault(); void save(); }}>
        <div className="panel-body tunnel-form-grid">
          <div className="field span-2"><label htmlFor="tunnel-name">规则名称</label><input id="tunnel-name" className="input" required value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })} placeholder="例如：生产数据库" /></div>
          <div className="field span-2"><label htmlFor="tunnel-server">SSH 服务器</label><select id="tunnel-server" className="select" required value={form.serverId} onChange={(event) => setForm({ ...form, serverId: event.target.value })}>{servers.map((server) => <option key={server.id} value={server.id}>{server.name} · {server.host}</option>)}</select></div>
          <div className="field span-2"><label htmlFor="tunnel-kind">转发类型</label><select id="tunnel-kind" className="select" value={form.kind} onChange={(event) => { const kind = event.target.value as PortForwardInput["kind"]; setForm({ ...form, kind, destinationHost: kind === "dynamic" ? "" : form.destinationHost || "127.0.0.1", destinationPort: kind === "dynamic" ? 0 : form.destinationPort || 80 }); }}><option value="local">本地转发 · 访问 VPS 侧服务</option><option value="remote">远程转发 · 暴露本机服务到 VPS</option><option value="dynamic">动态转发 · SOCKS5 代理</option></select></div>
          <div className="field"><label htmlFor="tunnel-listen-address">{form.kind === "remote" ? "VPS 监听地址" : "本地监听地址"}</label><input id="tunnel-listen-address" className="input" required value={form.listenAddress} onChange={(event) => setForm({ ...form, listenAddress: event.target.value })} /></div>
          <div className="field"><label htmlFor="tunnel-listen-port">{form.kind === "remote" ? "VPS 监听端口" : "本地端口"}（0 自动分配）</label><input id="tunnel-listen-port" className="input" type="number" min={0} max={65535} required value={form.listenPort} onChange={(event) => setForm({ ...form, listenPort: Number(event.target.value) })} /></div>
          {form.kind !== "dynamic" && <><div className="field"><label htmlFor="tunnel-destination-host">目标地址（{form.kind === "remote" ? "本机" : "VPS 视角"}）</label><input id="tunnel-destination-host" className="input" required value={form.destinationHost} onChange={(event) => setForm({ ...form, destinationHost: event.target.value })} /></div>
          <div className="field"><label htmlFor="tunnel-destination-port">目标端口</label><input id="tunnel-destination-port" className="input" type="number" min={1} max={65535} required value={form.destinationPort} onChange={(event) => setForm({ ...form, destinationPort: Number(event.target.value) })} /></div></>}
          <div className="span-2 tunnel-form-actions"><p>{form.kind === "dynamic" ? "启动后，在客户端 SOCKS5 代理中填写上述监听地址和端口。当前支持 CONNECT，不支持 UDP。" : form.kind === "remote" ? "VPS 收到的连接将通过 SSH 转发到本机目标。请确认目标服务仅提供你预期共享的内容。" : "目标地址从 VPS 视角解析。"} 建议使用 127.0.0.1，仅允许本机访问。</p><button className="btn btn-primary" type="submit" disabled={busy !== "" || !form.name.trim() || !form.serverId || form.listenPort < 0 || (form.kind !== "dynamic" && !form.destinationPort)}><Plus size={16} />保存规则</button></div>
        </div>
      </form>}
    </section>
    <section className="tunnel-rules">
      <div className="section-heading"><h2>转发规则</h2><span className="table-meta">{statuses.length} 条活动隧道</span></div>
      {loading ? <div className="skeleton" /> : rules.length === 0 ? <div className="empty"><div><Network size={30} /><h2>还没有转发规则</h2><p>保存一条规则后，可随时启动或停止，无需重复填写端口。</p></div></div> : <div className="table-wrap panel"><table className="data-table"><thead><tr><th>规则</th><th>监听</th><th>目标</th><th>连接</th><th>状态</th><th className="text-right">操作</th></tr></thead><tbody>{rules.map((rule) => {
        const status = statusMap.get(rule.id);
        const server = servers.find((item) => item.id === rule.serverId);
        return <tr key={rule.id}><td><span className="table-name">{rule.name}</span><span className="table-meta">{rule.kind === "dynamic" ? "动态 SOCKS5" : rule.kind === "remote" ? "远程转发" : "本地转发"} · {server?.name ?? "未知 VPS"}</span></td><td className="measured">{status?.localEndpoint ?? `${rule.listenAddress}:${rule.listenPort}`}</td><td className="measured">{rule.kind === "dynamic" ? "由客户端请求决定" : `${rule.destinationHost}:${rule.destinationPort}`}</td><td>{status ? <span className="table-meta">活动 {status.activeConnections} · 累计 {status.totalConnections}</span> : "—"}</td><td>{status ? <><span className="badge badge-ok">运行中</span><span className="table-meta">{formatTime(status.startedAt)} 启动</span></> : <span className="badge">已停止</span>}</td><td><div className="row-actions">{status && rule.kind === "local" && <button className="icon-btn" type="button" title="作为 HTTP 服务打开" aria-label={`打开 ${rule.name}`} disabled={busy !== ""} onClick={() => void openExternal(`http://${status.localEndpoint}`)}><ExternalLink size={15} /></button>}{status ? <button className="icon-btn" type="button" title="停止隧道" aria-label={`停止 ${rule.name}`} disabled={busy !== ""} onClick={() => void stop(rule)}><Square size={15} /></button> : <button className="icon-btn" type="button" title="启动隧道" aria-label={`启动 ${rule.name}`} disabled={busy !== ""} onClick={() => void start(rule)}><Play size={16} /></button>}<button className="icon-btn" type="button" title="删除规则" aria-label={`删除 ${rule.name}`} disabled={busy !== ""} onClick={() => void remove(rule, Boolean(status))}><Trash2 size={16} /></button></div></td></tr>;
      })}</tbody></table></div>}
    </section>
  </>;
}
