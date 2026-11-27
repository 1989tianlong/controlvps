import { useEffect, useRef, useState, type ClipboardEvent } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import { listen } from "@tauri-apps/api/event";
import { CircleDot, RotateCcw, Trash2 } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import type { Server, TerminalSession, TrustedExecutor } from "../types";
import "@xterm/xterm/css/xterm.css";

interface Props {
  server: Server;
  trusted: TrustedExecutor;
}

export function TerminalView({ server, trusted }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const sessionRef = useRef<TerminalSession | null>(null);
  const serverRef = useRef(server);
  serverRef.current = server;
  const [status, setStatus] = useState("正在建立 SSH 会话");
  const [error, setError] = useState("");
  const [attempt, setAttempt] = useState(0);
  const [recordingId, setRecordingId] = useState("");

  const connectionKey = [
    server.id,
    server.host,
    server.port,
    server.username,
    server.authMethod,
    server.privateKeyPath ?? "",
    server.jumpServerId ?? "",
    (server.jumpChain ?? []).join(","),
    server.authProfileId ?? "",
    server.groupName ?? "",
    server.inheritPort,
    server.inheritUsername,
    server.inheritJump,
    server.inheritAuth,
  ].join("|");

  useEffect(() => {
    if (!containerRef.current) return;
    const currentServer = serverRef.current;
    let disposed = false;
    let session: TerminalSession | null = null;
    let unlistenOutput: (() => void) | undefined;
    let unlistenExit: (() => void) | undefined;
    let unlistenRecording: (() => void) | undefined;
    setError("");
    setStatus("正在建立 SSH 会话");
    setRecordingId("");
    const css = getComputedStyle(document.documentElement);
    const token = (name: string) => css.getPropertyValue(name).trim();
    const terminal = new Terminal({
      cursorBlink: !window.matchMedia("(prefers-reduced-motion: reduce)").matches,
      fontFamily: token("--font-outlier"),
      fontSize: 14,
      lineHeight: 1.35,
      scrollback: 5000,
      allowProposedApi: true,
      theme: {
        background: token("--color-paper-3"),
        foreground: token("--color-ink"),
        cursor: token("--color-accent"),
        selectionBackground: token("--color-rule-2"),
        black: token("--color-paper-3"),
        red: token("--color-bad"),
        green: token("--color-ok"),
        yellow: token("--color-accent"),
        blue: token("--color-cool"),
        magenta: token("--color-ink-2"),
        cyan: token("--color-cool"),
        white: token("--color-ink"),
      },
    });
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(containerRef.current);
    terminalRef.current = terminal;
    fit.fit();
    terminal.writeln(`\x1b[2mConnecting to ${currentServer.username}@${currentServer.host}:${currentServer.port}...\x1b[0m`);
    const dataDisposable = terminal.onData((data) => {
      if (session) void api.terminalInput(session.terminalId, data).catch((reason) => setError(errorMessage(reason)));
    });
    const resizeDisposable = terminal.onResize(({ cols, rows }) => {
      if (session) void api.terminalResize(session.terminalId, cols, rows).catch(() => undefined);
    });
    const observer = new ResizeObserver(() => fit.fit());
    observer.observe(containerRef.current);

    trusted(currentServer, (allow) => api.openTerminal(currentServer.id, allow))
      .then(async (opened) => {
        if (disposed) {
          await api.terminalClose(opened.terminalId).catch(() => undefined);
          return;
        }
        session = opened;
        sessionRef.current = opened;
        setStatus(`已连接 ${currentServer.host}`);
        const disposeOutput = await listen<string>(`terminal-output-${opened.terminalId}`, (event) => terminal.write(event.payload));
        if (disposed) {
          disposeOutput();
          await api.terminalClose(opened.terminalId).catch(() => undefined);
          return;
        }
        unlistenOutput = disposeOutput;
        const disposeExit = await listen<string>(`terminal-exit-${opened.terminalId}`, (event) => {
          setStatus(event.payload);
          terminal.writeln(`\r\n\x1b[2m${event.payload}\x1b[0m`);
        });
        if (disposed) {
          disposeExit();
          unlistenOutput?.();
          await api.terminalClose(opened.terminalId).catch(() => undefined);
          return;
        }
        unlistenExit = disposeExit;
        const disposeRecording = await listen<string>(`terminal-recording-stopped-${opened.terminalId}`, (event) => {
          setRecordingId("");
          setError(event.payload);
        });
        if (disposed) {
          disposeRecording();
          unlistenOutput?.();
          unlistenExit?.();
          await api.terminalClose(opened.terminalId).catch(() => undefined);
          return;
        }
        unlistenRecording = disposeRecording;
        fit.fit();
        void api.terminalResize(opened.terminalId, terminal.cols, terminal.rows).catch(() => undefined);
        terminal.focus();
      })
      .catch((reason) => {
        if (!disposed) {
          setError(errorMessage(reason));
          setStatus("连接失败");
        }
      });

    return () => {
      disposed = true;
      dataDisposable.dispose();
      resizeDisposable.dispose();
      observer.disconnect();
      unlistenOutput?.();
      unlistenExit?.();
      unlistenRecording?.();
      if (session) void api.terminalClose(session.terminalId).catch(() => undefined);
      sessionRef.current = null;
      terminal.dispose();
      terminalRef.current = null;
    };
  }, [connectionKey, trusted, attempt]);

  const toggleRecording = async () => {
    const session = sessionRef.current;
    if (!session) return;
    if (recordingId) {
      try {
        await api.stopTerminalRecording(session.terminalId);
        setRecordingId("");
      } catch (reason) {
        setError(errorMessage(reason));
      }
      return;
    }
    if (!window.confirm("录制会保存这台服务器的终端输出（不记录原始键盘输入，但屏幕回显内容仍可能包含敏感信息），文件以仅当前用户可读权限保存在本机。开始录制？")) return;
    try {
      const handle = await api.startTerminalRecording(session.terminalId);
      setRecordingId(handle.id);
    } catch (reason) {
      setError(errorMessage(reason));
    }
  };

  const guardPaste = (event: ClipboardEvent<HTMLDivElement>) => {
    const text = event.clipboardData.getData("text");
    if (!/[\r\n]/.test(text)) return;
    event.preventDefault();
    event.stopPropagation();
    const terminalId = sessionRef.current?.terminalId;
    if (!terminalId) return;
    const lines = text.split(/\r\n|\r|\n/).filter((line) => line.length > 0).length;
    if (!window.confirm(`剪贴板包含 ${lines} 行内容。粘贴到 ${server.name} 可能会立即执行多条命令，确认继续？`)) return;
    void api.terminalInput(terminalId, text).catch((reason) => setError(errorMessage(reason)));
  };

  return (
    <div>
      <div className="panel-header">
        <div><span className="table-name">{server.username}@{server.host}</span><span className="table-meta">{status}</span></div>
        <div className="row-actions">
          {recordingId && <span className="badge badge-bad record-badge"><CircleDot size={13} />录制中</span>}
          <button className="icon-btn" type="button" title={recordingId ? "停止录制会话" : "录制会话"} aria-label={recordingId ? "停止录制会话" : "录制会话"} aria-pressed={Boolean(recordingId)} onClick={() => void toggleRecording()}><CircleDot size={16} /></button>
          <button className="icon-btn" type="button" title="清空终端" aria-label="清空终端" onClick={() => terminalRef.current?.clear()}><Trash2 size={16} /></button>
          <button className="icon-btn" type="button" title="重新连接" aria-label="重新连接" onClick={() => setAttempt((value) => value + 1)}><RotateCcw size={16} /></button>
        </div>
      </div>
      {error && <div className="error-banner m-4" role="alert">{error}</div>}
      <div ref={containerRef} className="terminal-shell" aria-label={`${server.name} SSH 终端`} onPasteCapture={guardPaste} />
    </div>
  );
}
