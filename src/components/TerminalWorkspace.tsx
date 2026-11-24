import { lazy, Suspense, useEffect, useState } from "react";
import { Plus, TerminalSquare, X } from "lucide-react";
import type { Server, TrustedExecutor } from "../types";

const TerminalView = lazy(() => import("./TerminalView").then((module) => ({ default: module.TerminalView })));

export interface TerminalRequest {
  id: number;
  serverId: string;
}

interface WorkspaceTab {
  id: string;
  server: Server;
}

interface Props {
  servers: Server[];
  trusted: TrustedExecutor;
  request?: TerminalRequest;
}

export function TerminalWorkspace({ servers, trusted, request }: Props) {
  const [tabs, setTabs] = useState<WorkspaceTab[]>([]);
  const [activeId, setActiveId] = useState("");
  const [selectedServerId, setSelectedServerId] = useState(servers[0]?.id ?? "");

  useEffect(() => {
    if (!servers.some((server) => server.id === selectedServerId)) setSelectedServerId(servers[0]?.id ?? "");
  }, [servers, selectedServerId]);

  useEffect(() => {
    setTabs((current) => current.flatMap((tab) => {
      const server = servers.find((item) => item.id === tab.server.id);
      if (!server) return [];
      return server.updatedAt === tab.server.updatedAt ? [tab] : [{ ...tab, server }];
    }));
  }, [servers]);

  useEffect(() => {
    if (activeId && !tabs.some((tab) => tab.id === activeId)) setActiveId(tabs[0]?.id ?? "");
  }, [tabs, activeId]);

  useEffect(() => {
    if (!request) return;
    const server = servers.find((item) => item.id === request.serverId);
    if (!server) return;
    setTabs((current) => {
      const existing = current.find((tab) => tab.server.id === server.id);
      if (existing) {
        setActiveId(existing.id);
        return current;
      }
      const tab = { id: crypto.randomUUID(), server };
      setActiveId(tab.id);
      return [...current, tab];
    });
  }, [request]);

  const addSession = () => {
    const server = servers.find((item) => item.id === selectedServerId);
    if (!server) return;
    const tab = { id: crypto.randomUUID(), server };
    setTabs((current) => [...current, tab]);
    setActiveId(tab.id);
  };

  const closeSession = (id: string) => {
    setTabs((current) => {
      const index = current.findIndex((tab) => tab.id === id);
      const remaining = current.filter((tab) => tab.id !== id);
      if (id === activeId) setActiveId(remaining[Math.min(index, remaining.length - 1)]?.id ?? "");
      return remaining;
    });
  };

  return <>
    <div className="page-heading"><div><h1>终端工作区</h1><p>在多个 SSH 会话间切换，离开此页面不会断开连接。</p></div><div className="heading-actions"><select className="select" aria-label="新建终端服务器" value={selectedServerId} onChange={(event) => setSelectedServerId(event.target.value)}>{servers.map((server) => <option key={server.id} value={server.id}>{server.name} · {server.username}@{server.host}</option>)}</select><button className="btn btn-primary" type="button" onClick={addSession} disabled={!selectedServerId}><Plus size={16} />新建会话</button></div></div>
    {servers.length === 0 ? <div className="empty"><div><TerminalSquare size={32} /><h2>先添加一台 VPS</h2><p>保存连接信息后，即可在这里打开交互式 SSH 终端。</p></div></div> : tabs.length === 0 ? <div className="empty"><div><TerminalSquare size={32} /><h2>工作区还没有会话</h2><p>选择服务器并新建会话，也可以从“我的 VPS”直接打开。</p><button className="btn btn-primary" type="button" onClick={addSession}><Plus size={16} />新建第一个会话</button></div></div> : <section className="panel terminal-workspace">
      <div className="workspace-tabs" role="tablist" aria-label="SSH 会话">{tabs.map((tab) => {
        const sessionNumber = tabs.filter((item) => item.server.id === tab.server.id).findIndex((item) => item.id === tab.id) + 1;
        return <div className="workspace-tab" data-active={tab.id === activeId} key={tab.id}><button type="button" role="tab" aria-selected={tab.id === activeId} aria-controls={`terminal-panel-${tab.id}`} onClick={() => setActiveId(tab.id)}><TerminalSquare size={15} /><span>{tab.server.name}<small>{sessionNumber > 1 ? `会话 ${sessionNumber}` : tab.server.host}</small></span></button><button className="workspace-close" type="button" aria-label={`关闭 ${tab.server.name} 会话 ${sessionNumber}`} onClick={() => closeSession(tab.id)}><X size={14} /></button></div>;
      })}<span className="workspace-count">{tabs.length} 个会话</span></div>
      {tabs.map((tab) => <div id={`terminal-panel-${tab.id}`} role="tabpanel" key={tab.id} hidden={tab.id !== activeId}><Suspense fallback={<div className="terminal-empty">正在加载终端组件</div>}><TerminalView server={tab.server} trusted={trusted} /></Suspense></div>)}
    </section>}
  </>;
}
