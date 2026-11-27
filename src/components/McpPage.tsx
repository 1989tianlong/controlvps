import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Copy, KeyRound, PlugZap, RefreshCw, Save, ShieldAlert, Trash2 } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import { formatTime } from "../lib/format";
import type { McpCallRecord, McpSettingsInput, McpStatus } from "../types";

export function McpPage() {
  const [status, setStatus] = useState<McpStatus>();
  const [draft, setDraft] = useState<McpSettingsInput>();
  const [calls, setCalls] = useState<McpCallRecord[]>([]);
  const [revealed, setRevealed] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");

  useEffect(() => {
    void api.mcpStatus()
      .then((value) => {
        setStatus(value);
        setDraft({
          enabled: value.enabled,
          port: value.port,
          allowExecute: value.allowExecute,
          allowUnknownHost: value.allowUnknownHost,
        });
      })
      .catch((reason) => setError(errorMessage(reason)));
  }, []);

  const loadCalls = useCallback(async () => {
    try {
      setCalls(await api.mcpCalls(30));
    } catch {
      // 调用记录读取失败不阻塞主界面
    }
  }, []);

  useEffect(() => { void loadCalls(); }, [loadCalls]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen("mcp-call", () => { void loadCalls(); }).then((dispose) => {
      if (disposed) dispose();
      else unlisten = dispose;
    }).catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [loadCalls]);

  const applyStatus = (value: McpStatus) => {
    setStatus(value);
    setDraft({
      enabled: value.enabled,
      port: value.port,
      allowExecute: value.allowExecute,
      allowUnknownHost: value.allowUnknownHost,
    });
  };

  const save = async () => {
    if (!draft) return;
    if (!Number.isInteger(draft.port) || draft.port < 1024 || draft.port > 65535) {
      setError("端口需要在 1024 到 65535 之间");
      return;
    }
    setSaving(true);
    setError("");
    setNotice("");
    try {
      const saved = await api.updateMcpSettings(draft);
      applyStatus(saved);
      setNotice(saved.running ? `MCP 服务已启动：${saved.endpoint}` : "MCP 服务已停止");
      if (saved.error) setError(saved.error);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setSaving(false);
    }
  };

  const rotate = async () => {
    if (!window.confirm("重置令牌后，所有已配置的 AI 客户端都会失去访问权限，需要重新复制新令牌。继续吗？")) return;
    setError("");
    try {
      const rotated = await api.rotateMcpToken();
      applyStatus(rotated);
      setRevealed(true);
      setNotice("访问令牌已重置");
    } catch (reason) {
      setError(errorMessage(reason));
    }
  };

  const copy = async (value: string, label: string) => {
    setError("");
    try {
      await navigator.clipboard.writeText(value);
      setNotice(`${label}已复制到剪贴板`);
    } catch {
      setError("复制失败，请手动选中文本复制");
    }
  };

  const clearCalls = async () => {
    try {
      await api.clearMcpCalls();
      setCalls([]);
    } catch (reason) {
      setError(errorMessage(reason));
    }
  };

  const update = <K extends keyof McpSettingsInput>(key: K, value: McpSettingsInput[K]) => {
    setDraft((current) => current ? { ...current, [key]: value } : current);
    setNotice("");
  };

  const token = status?.token ?? "";
  const masked = token ? `${token.slice(0, 6)}…${token.slice(-6)}` : "";
  const endpoint = status?.endpoint ?? "";
  const claudeCommand = `claude mcp add --transport http controlvps ${endpoint} \\\n  --header "Authorization: Bearer ${token}"`;
  const cursorConfig = `{\n  "mcpServers": {\n    "controlvps": {\n      "url": "${endpoint}",\n      "headers": { "Authorization": "Bearer ${token}" }\n    }\n  }\n}`;

  return <>
    <div className="page-heading">
      <div>
        <h1>AI 接入</h1>
        <p>通过 MCP（Model Context Protocol）把本机 ControlVPS 的服务器清单与运维能力交给 Claude Code、Cursor 等 AI 客户端，凭据始终留在本方应用内。</p>
      </div>
      <button className="btn btn-primary" type="button" onClick={save} disabled={!draft || saving}>
        <Save size={16} />{saving ? "正在应用" : "保存并应用"}
      </button>
    </div>
    {error && <div className="error-banner" role="alert">{error}</div>}
    {notice && <div className="panel panel-body mb-6" role="status">{notice}</div>}

    <div className="mcp-status panel panel-body">
      <div>
        <span className={status?.running ? "badge badge-ok" : "badge"}>{status?.running ? "运行中" : "已停止"}</span>
        <p>{status?.running ? `仅监听本机回环地址 ${endpoint}` : "开启后才会监听本机端口，外部网络无法访问。"}</p>
      </div>
      <div className="mcp-status-meta">
        <span className="measured">{status?.tools.length ?? 0} 个工具</span>
        <span className="measured">{calls.length} 次调用</span>
      </div>
    </div>

    <div className="settings-list">
      {!draft ? <div className="skeleton settings-skeleton" /> : <>
        <div className="setting-row">
          <div><h2>本地 MCP 服务</h2><p>应用运行期间监听本机端口，提供标准 Streamable HTTP 端点，客户端不需要额外安装桥接程序。</p></div>
          <label className="switch-control"><input type="checkbox" checked={draft.enabled} onChange={(event) => update("enabled", event.target.checked)} /><span>{draft.enabled ? "已开启" : "已关闭"}</span></label>
        </div>
        <div className="setting-row">
          <div><h2>监听端口</h2><p>只绑定 127.0.0.1。更换端口后需要重新保存，并同步更新 AI 客户端配置。</p></div>
          <div className="setting-actions">
            <input className="input" type="number" min={1024} max={65535} value={draft.port} disabled={!draft.enabled} onChange={(event) => update("port", Number(event.target.value))} />
            <code className="measured">{endpoint}</code>
          </div>
        </div>
        <div className="setting-row">
          <div><h2>访问令牌</h2><p>每个请求都必须携带 Bearer 令牌。令牌只保存在本机，重置后旧客户端会立即失效。</p></div>
          <div className="setting-actions">
            <code className="measured mcp-token">{revealed ? token : masked}</code>
            <button className="btn" type="button" onClick={() => setRevealed((value) => !value)}><KeyRound size={15} />{revealed ? "隐藏" : "显示"}</button>
            <button className="btn" type="button" onClick={() => void copy(token, "访问令牌")}><Copy size={15} />复制</button>
            <button className="btn btn-danger" type="button" onClick={rotate}><RefreshCw size={15} />重置</button>
          </div>
        </div>
        <div className="setting-row">
          <div><h2>允许执行命令与变更</h2><p>开启后 AI 可以执行命令、批量命令和 Runbook，等同于把 root 级操作交给客户端。默认关闭，建议按需临时开启。</p></div>
          <label className="switch-control"><input type="checkbox" checked={draft.allowExecute} onChange={(event) => update("allowExecute", event.target.checked)} /><span>{draft.allowExecute ? "已允许" : "只读模式"}</span></label>
        </div>
        <div className="setting-row">
          <div><h2>自动信任未知主机</h2><p>关闭时，AI 连接未确认过主机密钥的服务器会被拒绝，需要你先在应用内手动连接一次。开启会跳过这层保护，不建议在生产环境使用。</p></div>
          <label className="switch-control"><input type="checkbox" checked={draft.allowUnknownHost} onChange={(event) => update("allowUnknownHost", event.target.checked)} /><span>{draft.allowUnknownHost ? "已开启" : "已关闭"}</span></label>
        </div>
      </>}
      <div className="setting-row">
        <div><h2>连接安全</h2><p>服务只监听回环地址，校验 Bearer 令牌与请求来源；密码和私钥仍由 macOS Keychain 保管，不会出现在工具返回内容里。</p></div>
        <div className="setting-value flex items-center gap-2"><ShieldAlert size={16} />本地回环 + 令牌校验</div>
      </div>
    </div>

    <section className="panel panel-body mcp-config">
      <div className="mcp-config-head">
        <h2><PlugZap size={16} />客户端配置</h2>
        <p>任何支持 Streamable HTTP 的 MCP 客户端都可以接入，无需安装额外程序。</p>
      </div>
      <div className="mcp-config-grid">
        <div>
          <div className="mcp-config-label"><span>Claude Code</span><button className="btn" type="button" onClick={() => void copy(claudeCommand, "Claude Code 命令")}><Copy size={14} />复制命令</button></div>
          <pre>{claudeCommand}</pre>
        </div>
        <div>
          <div className="mcp-config-label"><span>Cursor / 通用 mcp.json</span><button className="btn" type="button" onClick={() => void copy(cursorConfig, "MCP 配置")}><Copy size={14} />复制配置</button></div>
          <pre>{cursorConfig}</pre>
        </div>
      </div>
    </section>

    <section className="mcp-section">
      <div className="mcp-section-head">
        <h2>可用工具</h2>
        <p>写操作在“只读模式”下会被拒绝，并在调用记录里留下痕迹。</p>
      </div>
      <div className="table-wrap panel">
        <table className="data-table">
          <thead><tr><th>工具</th><th>权限</th><th>说明</th></tr></thead>
          <tbody>{(status?.tools ?? []).map((tool) => <tr key={tool.name}><td className="table-name"><code>{tool.name}</code><br /><span className="measured">{tool.title}</span></td><td><span className={tool.write ? "badge badge-bad" : "badge badge-cool"}>{tool.write ? "写" : "只读"}</span></td><td>{tool.description}</td></tr>)}</tbody>
        </table>
      </div>
    </section>

    <section className="mcp-section">
      <div className="mcp-section-head">
        <h2>最近调用</h2>
        <p>记录 AI 客户端发起的每一次工具调用，不含命令输出与凭据。</p>
        <div className="setting-actions">
          <button className="btn" type="button" onClick={() => void loadCalls()}><RefreshCw size={15} />刷新</button>
          <button className="btn btn-danger" type="button" onClick={clearCalls} disabled={calls.length === 0}><Trash2 size={15} />清空</button>
        </div>
      </div>
      {calls.length === 0 ? <div className="empty"><div><PlugZap size={30} /><h2>还没有 AI 调用</h2><p>配置客户端并让它读取服务器清单后，调用记录会出现在这里。</p></div></div> : <div className="table-wrap panel"><table className="data-table"><thead><tr><th>时间</th><th>工具</th><th>对象</th><th>结果</th><th>摘要</th><th>耗时</th></tr></thead><tbody>{calls.map((call) => <tr key={call.id}><td className="log-time">{formatTime(call.createdAt)}</td><td><code>{call.tool}</code></td><td>{call.target || "—"}</td><td><span className={call.success ? "badge badge-ok" : "badge badge-bad"}>{call.success ? "成功" : "失败"}</span></td><td>{call.summary}</td><td className="measured">{call.durationMs} ms</td></tr>)}</tbody></table></div>}
    </section>
  </>;
}
