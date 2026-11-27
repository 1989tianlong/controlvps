import { useCallback, useEffect, useState } from "react";
import { Activity, Bot, Cloud, Compass, Gift, LayoutDashboard, ListChecks, LockKeyhole, Menu, MonitorPlay, Network, Plus, ScrollText, Server as ServerIcon, Settings, ShieldCheck, TerminalSquare, X } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage } from "./lib/api";
import { useHostTrust } from "./hooks/useHostTrust";
import type { AlertEvent, MonitoringSettings, Server, ServerMetrics } from "./types";
import { useMonitoring } from "./hooks/useMonitoring";
import { useServiceMonitoring } from "./hooks/useServiceMonitoring";
import { sendSystemNotification } from "./lib/notifications";
import { ServerForm } from "./components/ServerForm";
import { OverviewPage } from "./components/OverviewPage";
import { ServersPage } from "./components/ServersPage";
import { AssetsPage } from "./components/AssetsPage";
import { DiscoveryPage } from "./components/DiscoveryPage";
import { FreePage } from "./components/FreePage";
import { ActivityPage } from "./components/ActivityPage";
import { SettingsPage } from "./components/SettingsPage";
import { AutomationPage } from "./components/AutomationPage";
import { TunnelsPage } from "./components/TunnelsPage";
import { MonitoringPage } from "./components/MonitoringPage";
import { SecurityCenter } from "./components/SecurityCenter";
import { TerminalWorkspace, type TerminalRequest } from "./components/TerminalWorkspace";
import { RecordingsPage } from "./components/RecordingsPage";
import { McpPage } from "./components/McpPage";
import { UpdateBanner } from "./components/UpdateBanner";
import { useAppUpdate } from "./hooks/useAppUpdate";

const pages = [
  { id: "overview", label: "总览", icon: LayoutDashboard },
  { id: "servers", label: "我的 VPS", icon: ServerIcon },
  { id: "monitoring", label: "监控告警", icon: Activity },
  { id: "security", label: "补丁与安全", icon: ShieldCheck },
  { id: "terminal", label: "终端工作区", icon: TerminalSquare },
  { id: "recordings", label: "录制回放", icon: MonitorPlay },
  { id: "automation", label: "批量运维", icon: ListChecks },
  { id: "tunnels", label: "隧道中心", icon: Network },
  { id: "assets", label: "云资产", icon: Cloud },
  { id: "discover", label: "发现 VPS", icon: Compass },
  { id: "free", label: "免费渠道", icon: Gift },
  { id: "activity", label: "操作记录", icon: ScrollText },
  { id: "mcp", label: "AI 接入", icon: Bot },
  { id: "settings", label: "设置", icon: Settings },
] as const;

type Page = (typeof pages)[number]["id"];

function pageFromHash(): Page {
  const value = window.location.hash.replace(/^#\/?/, "");
  return pages.some((page) => page.id === value) ? value as Page : "overview";
}

export default function App() {
  const [page, setPage] = useState<Page>(pageFromHash);
  const [menuOpen, setMenuOpen] = useState(false);
  const [servers, setServers] = useState<Server[]>([]);
  const [metrics, setMetrics] = useState<Record<string, ServerMetrics>>({});
  const [monitoringSettings, setMonitoringSettings] = useState<MonitoringSettings>();
  const [terminalRequest, setTerminalRequest] = useState<TerminalRequest>();
  const [automationMode, setAutomationMode] = useState<"snippets" | "runbooks" | "operations">("snippets");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [formOpen, setFormOpen] = useState(false);
  const [editing, setEditing] = useState<Server | null>(null);
  const { execute, trustDialog } = useHostTrust();
  const appUpdate = useAppUpdate();
  const updateMetrics = useCallback((id: string, value: ServerMetrics) => {
    setMetrics((current) => ({ ...current, [id]: value }));
  }, []);
  const { errors: monitorErrors, collecting } = useMonitoring(servers, monitoringSettings, updateMetrics);
  const serviceMonitoring = useServiceMonitoring(monitoringSettings?.notificationsEnabled ?? false);

  const loadServers = useCallback(async () => {
    setError("");
    try { setServers(await api.listServers()); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setLoading(false); }
  }, []);

  useEffect(() => {
    void loadServers();
    const onHash = () => setPage(pageFromHash());
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, [loadServers]);

  useEffect(() => {
    void api.monitoringSettings().then(setMonitoringSettings).catch((reason) => setError(errorMessage(reason)));
  }, []);

  useEffect(() => {
    if (!monitoringSettings?.notificationsEnabled) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<AlertEvent>("monitoring-alert", (event) => {
      if (event.payload.status !== "triggered") return;
      void sendSystemNotification(`ControlVPS · ${event.payload.serverName}`, event.payload.message).catch(() => undefined);
    }).then((dispose) => {
      if (disposed) dispose();
      else unlisten = dispose;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [monitoringSettings?.notificationsEnabled]);

  const navigate = (target: Page) => {
    window.location.hash = `/${target}`;
    setPage(target);
    setMenuOpen(false);
  };
  const add = () => { setEditing(null); setFormOpen(true); };
  const edit = (server: Server) => { setEditing(server); setFormOpen(true); };
  const openTerminal = (server: Server) => {
    setTerminalRequest((current) => ({ id: (current?.id ?? 0) + 1, serverId: server.id }));
    navigate("terminal");
  };
  const saved = (savedServer: Server) => {
    setServers((current) => {
      const exists = current.some((server) => server.id === savedServer.id);
      return exists ? current.map((server) => server.id === savedServer.id ? savedServer : server) : [...current, savedServer];
    });
  };

  return <div className="app-shell">
    {menuOpen && <button className="mobile-backdrop" type="button" aria-label="关闭导航" onClick={() => setMenuOpen(false)} />}
    <aside className="sidebar" data-open={menuOpen}>
      <div className="brand"><span className="brand-mark"><TerminalSquare size={19} /></span><span>ControlVPS</span></div>
      <nav className="sidebar-nav" aria-label="主导航">{pages.map((item) => <button className="nav-item" type="button" key={item.id} aria-current={page === item.id ? "page" : undefined} onClick={() => navigate(item.id)}><item.icon size={18} /><span>{item.label}</span></button>)}</nav>
      <button className="btn btn-primary mt-6" type="button" onClick={add}><Plus size={16} />添加 VPS</button>
      <div className="sidebar-foot"><LockKeyhole size={17} /><p>本地优先，凭据由 macOS Keychain 保护。</p></div>
    </aside>
    <main className="main-column">
      <header className="topbar"><div className="flex items-center gap-3"><button className="icon-btn menu-button" type="button" aria-label={menuOpen ? "关闭导航" : "打开导航"} onClick={() => setMenuOpen((value) => !value)}>{menuOpen ? <X size={19} /> : <Menu size={19} />}</button><span className="topbar-title">{pages.find((item) => item.id === page)?.label}</span></div><span className="local-label">{collecting || serviceMonitoring.checkingIds.length > 0 ? <Activity size={14} /> : <LockKeyhole size={14} />}{collecting || serviceMonitoring.checkingIds.length > 0 ? "正在采集监控" : "本地工作区"}</span></header>
      <div className="page">
        <UpdateBanner state={appUpdate.state} dismissed={appUpdate.dismissed} onInstall={() => void appUpdate.install()} onRestart={() => void appUpdate.restart()} onDismiss={appUpdate.dismiss} />
        {error && <div className="error-banner" role="alert"><p>{error}</p><button className="btn" type="button" onClick={loadServers}>重试</button></div>}
        {page === "overview" && <OverviewPage servers={servers} loading={loading} metrics={metrics} onAdd={add} onServers={() => navigate("servers")} onDiscover={() => navigate("discover")} />}
        {page === "servers" && <ServersPage servers={servers} loading={loading} metrics={metrics} onMetrics={updateMetrics} onAdd={add} onEdit={edit} onChanged={loadServers} onOpenTerminal={openTerminal} trusted={execute} />}
        {page === "monitoring" && <MonitoringPage servers={servers} metrics={metrics} errors={monitorErrors} settings={monitoringSettings} trusted={execute} onMetrics={updateMetrics} onSettings={() => navigate("settings")} serviceMonitoring={serviceMonitoring} />}
        {page === "security" && <SecurityCenter servers={servers} trusted={execute} onAdd={add} onOpenRunbooks={() => { setAutomationMode("runbooks"); navigate("automation"); }} />}
        {page === "automation" && <AutomationPage servers={servers} trusted={execute} onAdd={add} requestedMode={automationMode} />}
        {page === "tunnels" && <TunnelsPage servers={servers} trusted={execute} onAdd={add} />}
        <div hidden={page !== "terminal"}><TerminalWorkspace servers={servers} trusted={execute} request={terminalRequest} /></div>
        {page === "recordings" && <RecordingsPage />}
        {page === "assets" && <AssetsPage servers={servers} onChanged={loadServers} />}
        {page === "discover" && <DiscoveryPage />}
        {page === "free" && <FreePage />}
        {page === "activity" && <ActivityPage />}
        {page === "mcp" && <McpPage />}
        {page === "settings" && <SettingsPage settings={monitoringSettings} onSettingsChange={setMonitoringSettings} appUpdate={appUpdate} onWorkspaceRestored={() => { void loadServers(); void serviceMonitoring.refresh(); void api.monitoringSettings().then(setMonitoringSettings).catch(() => undefined); }} />}
      </div>
    </main>
    <ServerForm server={editing} servers={servers} open={formOpen} onClose={() => setFormOpen(false)} onSaved={saved} />
    {trustDialog}
  </div>;
}
