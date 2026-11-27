import { useEffect, useMemo, useState } from "react";
import { Cable, Container, FileInput, FolderOpen, Pencil, Plus, RefreshCw, Search, Server as ServerIcon, Settings2, TerminalSquare, Trash2 } from "lucide-react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api, errorMessage } from "../lib/api";
import type { ConnectionReport, Server, ServerMetrics, TrustedExecutor } from "../types";
import { ServerDetail, type DetailTab } from "./ServerDetail";
import { jumpChainOf, jumpServersOf } from "../lib/route";

interface Props {
  servers: Server[];
  loading: boolean;
  metrics: Record<string, ServerMetrics>;
  onMetrics: (serverId: string, metrics: ServerMetrics) => void;
  onAdd: () => void;
  onEdit: (server: Server) => void;
  onChanged: () => Promise<void>;
  onOpenTerminal: (server: Server) => void;
  onOpenRunbooks?: () => void;
  trusted: TrustedExecutor;
}

export function ServersPage({ servers, loading, metrics, onMetrics, onAdd, onEdit, onChanged, onOpenTerminal, onOpenRunbooks, trusted }: Props) {
  const [query, setQuery] = useState("");
  const [group, setGroup] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [tab, setTab] = useState<DetailTab>("overview");
  const [busy, setBusy] = useState<string | null>(null);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [diagnostics, setDiagnostics] = useState<Record<string, ConnectionReport>>({});
  const [notice, setNotice] = useState("");
  const groups = useMemo(() => [...new Set(servers.map((server) => server.groupName).filter((value): value is string => Boolean(value)))], [servers]);
  const filtered = servers.filter((server) => {
    const text = `${server.name} ${server.host} ${server.provider ?? ""} ${server.tags.join(" ")}`.toLowerCase();
    return text.includes(query.toLowerCase()) && (!group || server.groupName === group);
  });
  const selected = servers.find((server) => server.id === selectedId);

  useEffect(() => {
    if (selectedId && !servers.some((server) => server.id === selectedId)) setSelectedId(null);
  }, [servers, selectedId]);

  const refresh = async (server: Server, showBusy = true) => {
    if (showBusy) setBusy(server.id);
    setErrors((current) => ({ ...current, [server.id]: "" }));
    try {
      const result = await trusted(server, (allow) => api.collectMetrics(server.id, allow));
      onMetrics(server.id, result);
    } catch (reason) {
      setErrors((current) => ({ ...current, [server.id]: errorMessage(reason) }));
    } finally {
      if (showBusy) setBusy(null);
    }
  };

  const test = async (server: Server) => {
    setBusy(server.id);
    setNotice("");
    setErrors((current) => ({ ...current, [server.id]: "" }));
    setSelectedId(server.id);
    setTab("diagnostics");
    try {
      const result = await trusted(server, (allow) => api.diagnoseConnection(server.id, allow));
      setDiagnostics((current) => ({ ...current, [server.id]: result }));
      setNotice(`${server.name}：${result.message}，${result.latencyMs} ms`);
    } catch (reason) {
      setErrors((current) => ({ ...current, [server.id]: errorMessage(reason) }));
    } finally {
      setBusy(null);
    }
  };

  const importConfig = async () => {
    setNotice("");
    try {
      const path = await openDialog({ title: "选择 SSH Config 文件", multiple: false, directory: false });
      if (!path) return;
      setBusy("import");
      const result = await api.importSshConfig(path);
      await onChanged();
      setNotice(`已导入 ${result.imported.length} 台 VPS，跳过 ${result.skipped} 条。${result.warnings.length ? ` ${result.warnings.slice(0, 3).join("；")}` : ""}`);
    } catch (reason) {
      setNotice(errorMessage(reason));
    } finally {
      setBusy(null);
    }
  };

  const remove = async (server: Server) => {
    if (!window.confirm(`确认删除 ${server.name}？本地配置和钥匙串凭据会被移除，远程服务器不受影响。`)) return;
    try {
      await api.deleteServer(server.id);
      await onChanged();
    } catch (reason) {
      setNotice(errorMessage(reason));
    }
  };

  const open = (server: Server, nextTab: DetailTab) => {
    setSelectedId(server.id);
    setTab(nextTab);
  };

  return <>
    <div className="page-heading"><div><h1>我的 VPS</h1><p>连接、监控并操作你自己的服务器。</p></div><div className="heading-actions"><button className="btn" type="button" disabled={busy !== null} onClick={() => void importConfig()}><FileInput size={16} />{busy === "import" ? "正在导入" : "导入 SSH Config"}</button><button className="btn btn-primary" type="button" onClick={onAdd}><Plus size={16} />添加 VPS</button></div></div>
    {notice && <div className="panel panel-body mb-6" role="status">{notice}</div>}
    <div className="filters">
      <div className="search-wrap"><Search size={16} /><input className="input" type="search" aria-label="搜索服务器" placeholder="搜索名称、地址或标签" value={query} onChange={(e) => setQuery(e.target.value)} /></div>
      <select className="select" aria-label="按分组筛选" value={group} onChange={(e) => setGroup(e.target.value)}><option value="">全部分组</option>{groups.map((item) => <option key={item}>{item}</option>)}</select>
    </div>
    {loading ? <div className="skeleton" aria-label="正在加载服务器" /> : servers.length === 0 ? <div className="empty"><div><ServerIcon size={32} /><h2>还没有 VPS</h2><p>添加服务器后，即可使用真实 SSH 终端和监控。</p><button className="btn btn-primary" type="button" onClick={onAdd}><Plus size={16} />添加第一台 VPS</button></div></div> : filtered.length === 0 ? <div className="empty"><div><Search size={30} /><h2>没有匹配的服务器</h2><p>清除搜索词或选择其他分组。</p><button className="btn" type="button" onClick={() => { setQuery(""); setGroup(""); }}>清除筛选</button></div></div> : <div className="table-wrap panel">
      <table className="data-table"><thead><tr><th>服务器</th><th>分组</th><th>状态</th><th>CPU / 内存</th><th>认证</th><th className="text-right">操作</th></tr></thead><tbody>
        {filtered.map((server) => {
          const reading = metrics[server.id];
          const error = errors[server.id];
          return <tr key={server.id}>
            <td><button className="table-link" type="button" onClick={() => open(server, "overview")}><span className="table-name">{server.name}</span><span className="table-meta measured">{server.host}:{server.port}</span></button></td>
            <td>{server.groupName || "未分组"}</td>
            <td><span className={reading ? "badge badge-ok" : error ? "badge badge-bad" : "badge"}>{reading ? "在线" : error ? "连接失败" : "未检查"}</span>{error && <span className="table-meta" title={error}>查看详情</span>}</td>
            <td className="measured">{reading ? `${reading.cpuUsage.toFixed(1)}% / ${reading.memoryTotal ? (reading.memoryUsed / reading.memoryTotal * 100).toFixed(1) : 0}%` : "无读数"}</td>
            <td>{{ key: "私钥", password: "密码", agent: "Agent" }[server.authMethod]}{jumpChainOf(server).length > 0 && <span className="table-meta">经 {jumpChainOf(server).length} 级跳板</span>}</td>
            <td><div className="row-actions">
              <button className="icon-btn" type="button" title="测试连接" aria-label={`测试 ${server.name}`} disabled={busy === server.id} onClick={() => test(server)}><Cable size={16} /></button>
              <button className="icon-btn" type="button" title="刷新指标" aria-label={`刷新 ${server.name} 指标`} disabled={busy === server.id} onClick={() => refresh(server)}><RefreshCw size={16} /></button>
              <button className="icon-btn" type="button" title="打开终端" aria-label={`打开 ${server.name} 终端`} onClick={() => onOpenTerminal(server)}><TerminalSquare size={16} /></button>
              <button className="icon-btn" type="button" title="管理文件" aria-label={`管理 ${server.name} 文件`} onClick={() => open(server, "files")}><FolderOpen size={16} /></button>
              <button className="icon-btn" type="button" title="Docker 容器" aria-label={`管理 ${server.name} Docker 容器`} onClick={() => open(server, "docker")}><Container size={16} /></button>
              <button className="icon-btn" type="button" title="systemd 服务" aria-label={`管理 ${server.name} systemd 服务`} onClick={() => open(server, "services")}><Settings2 size={16} /></button>
              <button className="icon-btn" type="button" title="编辑" aria-label={`编辑 ${server.name}`} onClick={() => onEdit(server)}><Pencil size={16} /></button>
              <button className="icon-btn" type="button" title="删除" aria-label={`删除 ${server.name}`} onClick={() => remove(server)}><Trash2 size={16} /></button>
            </div></td>
          </tr>;
        })}
      </tbody></table>
    </div>}
    {selected && <ServerDetail server={selected} jumpServers={jumpServersOf(selected, servers)} diagnostics={diagnostics[selected.id]} onDiagnose={() => void test(selected)} metrics={metrics[selected.id]} metricsError={errors[selected.id]} tab={tab} onTabChange={setTab} onRefresh={() => refresh(selected)} refreshing={busy === selected.id} trusted={trusted} onOpenRunbooks={onOpenRunbooks} />}
  </>;
}
