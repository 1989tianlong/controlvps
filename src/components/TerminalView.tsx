import { useEffect, useRef, useState } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import { listen } from "@tauri-apps/api/event";
import { RotateCcw, Trash2 } from "lucide-react";
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
  const [status, setStatus] = useState("正在建立 SSH 会话");
  const [error, setError] = useState("");
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    if (!containerRef.current) return;
    let disposed = false;
    let session: TerminalSession | null = null;
    let unlistenOutput: (() => void) | undefined;
    let unlistenExit: (() => void) | undefined;
    setError("");
    setStatus("正在建立 SSH 会话");
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
    terminal.writeln(`\x1b[2mConnecting to ${server.username}@${server.host}:${server.port}...\x1b[0m`);
    const dataDisposable = terminal.onData((data) => {
      if (session) void api.terminalInput(session.terminalId, data).catch((reason) => setError(errorMessage(reason)));
    });
    const resizeDisposable = terminal.onResize(({ cols, rows }) => {
      if (session) void api.terminalResize(session.terminalId, cols, rows).catch(() => undefined);
    });
    const observer = new ResizeObserver(() => fit.fit());
    observer.observe(containerRef.current);

    trusted(server, (allow) => api.openTerminal(server.id, allow))
      .then(async (opened) => {
        if (disposed) {
          await api.terminalClose(opened.terminalId).catch(() => undefined);
          return;
        }
        session = opened;
        setStatus(`已连接 ${server.host}`);
        unlistenOutput = await listen<string>(`terminal-output-${opened.terminalId}`, (event) => terminal.write(event.payload));
        unlistenExit = await listen<string>(`terminal-exit-${opened.terminalId}`, (event) => {
          setStatus(event.payload);
          terminal.writeln(`\r\n\x1b[2m${event.payload}\x1b[0m`);
        });
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
      if (session) void api.terminalClose(session.terminalId).catch(() => undefined);
      terminal.dispose();
      terminalRef.current = null;
    };
  }, [server, trusted, attempt]);

  return (
    <div>
      <div className="panel-header">
        <div><span className="table-name">{server.username}@{server.host}</span><span className="table-meta">{status}</span></div>
        <div className="row-actions">
          <button className="icon-btn" type="button" title="清空终端" aria-label="清空终端" onClick={() => terminalRef.current?.clear()}><Trash2 size={16} /></button>
          <button className="icon-btn" type="button" title="重新连接" aria-label="重新连接" onClick={() => setAttempt((value) => value + 1)}><RotateCcw size={16} /></button>
        </div>
      </div>
      {error && <div className="error-banner m-4" role="alert">{error}</div>}
      <div ref={containerRef} className="terminal-shell" aria-label={`${server.name} SSH 终端`} />
    </div>
  );
}
