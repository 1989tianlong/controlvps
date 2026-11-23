import { useCallback, useEffect, useState } from "react";
import { Compass, Gift, LayoutDashboard, LockKeyhole, Menu, Plus, ScrollText, Server as ServerIcon, Settings, TerminalSquare, X } from "lucide-react";
import { api, errorMessage } from "./lib/api";
import { useHostTrust } from "./hooks/useHostTrust";
import type { Server, ServerMetrics } from "./types";
import { ServerForm } from "./components/ServerForm";
import { OverviewPage } from "./components/OverviewPage";
import { ServersPage } from "./components/ServersPage";
import { DiscoveryPage } from "./components/DiscoveryPage";
import { FreePage } from "./components/FreePage";
import { ActivityPage } from "./components/ActivityPage";
import { SettingsPage } from "./components/SettingsPage";

const pages = [
  { id: "overview", label: "总览", icon: LayoutDashboard },
  { id: "servers", label: "我的 VPS", icon: ServerIcon },
  { id: "discover", label: "发现 VPS", icon: Compass },
  { id: "free", label: "免费渠道", icon: Gift },
  { id: "activity", label: "操作记录", icon: ScrollText },
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
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [formOpen, setFormOpen] = useState(false);
  const [editing, setEditing] = useState<Server | null>(null);
  const { execute, trustDialog } = useHostTrust();

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

  const navigate = (target: Page) => {
    window.location.hash = `/${target}`;
    setPage(target);
    setMenuOpen(false);
  };
  const add = () => { setEditing(null); setFormOpen(true); };
  const edit = (server: Server) => { setEditing(server); setFormOpen(true); };
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
      <header className="topbar"><div className="flex items-center gap-3"><button className="icon-btn menu-button" type="button" aria-label={menuOpen ? "关闭导航" : "打开导航"} onClick={() => setMenuOpen((value) => !value)}>{menuOpen ? <X size={19} /> : <Menu size={19} />}</button><span className="topbar-title">{pages.find((item) => item.id === page)?.label}</span></div><span className="local-label"><LockKeyhole size={14} />本地工作区</span></header>
      <div className="page">
        {error && <div className="error-banner" role="alert"><p>{error}</p><button className="btn" type="button" onClick={loadServers}>重试</button></div>}
        {page === "overview" && <OverviewPage servers={servers} loading={loading} metrics={metrics} onAdd={add} onServers={() => navigate("servers")} onDiscover={() => navigate("discover")} />}
        {page === "servers" && <ServersPage servers={servers} loading={loading} metrics={metrics} onMetrics={(id, value) => setMetrics((current) => ({ ...current, [id]: value }))} onAdd={add} onEdit={edit} onChanged={loadServers} trusted={execute} />}
        {page === "discover" && <DiscoveryPage />}
        {page === "free" && <FreePage />}
        {page === "activity" && <ActivityPage />}
        {page === "settings" && <SettingsPage />}
      </div>
    </main>
    <ServerForm server={editing} open={formOpen} onClose={() => setFormOpen(false)} onSaved={saved} />
    {trustDialog}
  </div>;
}
