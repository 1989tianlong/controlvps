import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Activity, Pause, Play, RefreshCw, ScrollText, Search, Square, Trash2 } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import { formatBytes } from "../lib/format";
import type {
  LogStreamClosedEvent,
  LogStreamDataEvent,
  LogStreamSource,
  ProcessEntry,
  Server,
  TrustedExecutor,
} from "../types";

interface Props {
  server: Server;
  trusted: TrustedExecutor;
}

type LogMode = "systemd" | "docker" | "file";
type SortBy = "cpu" | "mem" | "pid";

const MAX_LOG_LINES = 2000;

export function LiveOpsPanel({ server, trusted }: Props) {
  const [subTab, setSubTab] = useState<"logs" | "processes">("logs");

  // Live Log Stream state
  const [logMode, setLogMode] = useState<LogMode>("systemd");
  const [unit, setUnit] = useState("");
  const [container, setContainer] = useState("");
  const [filePath, setFilePath] = useState("/var/log/syslog");
  const [initialLines, setInitialLines] = useState(100);
  const [streamId, setStreamId] = useState<string | null>(null);
  const [paused, setPaused] = useState(false);
  const [logFilter, setLogFilter] = useState("");
  const [logLines, setLogLines] = useState<string[]>([]);
  const [logStatus, setLogStatus] = useState("");
  const [logError, setLogError] = useState("");
  const [startingLog, setStartingLog] = useState(false);
  const streamIdRef = useRef<string | null>(null);
  const pausedRef = useRef(false);
  const logBoxRef = useRef<HTMLPreElement>(null);

  // Process Manager state
  const [processes, setProcesses] = useState<ProcessEntry[]>([]);
  const [procLoading, setProcLoading] = useState(false);
  const [procError, setProcError] = useState("");
  const [procNotice, setProcNotice] = useState("");
  const [procFilter, setProcFilter] = useState("");
  const [sortBy, setSortBy] = useState<SortBy>("cpu");
  const [autoRefresh, setAutoRefresh] = useState(false);
  const [signalingPid, setSignalingPid] = useState<number | null>(null);

  useEffect(() => {
    pausedRef.current = paused;
  }, [paused]);

  useEffect(() => {
    streamIdRef.current = streamId;
  }, [streamId]);

  useEffect(() => {
    let active = true;
    const unlisteners: (() => void)[] = [];
    void Promise.all([
      listen<LogStreamDataEvent>("log-stream-data", (event) => {
        if (!active || event.payload.streamId !== streamIdRef.current) return;
        const incoming = event.payload.chunk.replace(/\r\n/g, "\n").split("\n");
        setLogLines((prev) => {
          const merged = [...prev];
          if (merged.length > 0 && incoming.length > 0) {
            merged[merged.length - 1] += incoming[0];
            for (let i = 1; i < incoming.length; i += 1) {
              merged.push(incoming[i]);
            }
          } else {
            merged.push(...incoming);
          }
          return merged.length > MAX_LOG_LINES ? merged.slice(merged.length - MAX_LOG_LINES) : merged;
        });
      }),
      listen<LogStreamClosedEvent>("log-stream-closed", (event) => {
        if (!active || event.payload.streamId !== streamIdRef.current) return;
        setStreamId(null);
        setLogStatus(event.payload.reason);
      }),
    ]).then((fns) => {
      if (!active) {
        fns.forEach((fn) => fn());
      } else {
        unlisteners.push(...fns);
      }
    });
    return () => {
      active = false;
      unlisteners.forEach((fn) => fn());
      if (streamIdRef.current) {
        void api.stopLogStream(streamIdRef.current);
      }
    };
  }, [server.id]);

  useEffect(() => {
    if (!paused && logBoxRef.current) {
      logBoxRef.current.scrollTop = logBoxRef.current.scrollHeight;
    }
  }, [logLines, paused]);

  const startStream = async () => {
    setStartingLog(true);
    setLogError("");
    setLogStatus("");
    try {
      if (streamIdRef.current) {
        await api.stopLogStream(streamIdRef.current);
      }
      let source: LogStreamSource;
      if (logMode === "systemd") {
        source = { kind: "systemd", unit: unit.trim() || undefined, lines: initialLines };
      } else if (logMode === "docker") {
        if (!container.trim()) {
          throw new Error("请输入 Docker 容器名称或 ID");
        }
        source = { kind: "docker", container: container.trim(), lines: initialLines };
      } else {
        if (!filePath.trim()) {
          throw new Error("请输入日志文件路径");
        }
        source = { kind: "file", path: filePath.trim(), lines: initialLines };
      }
      setLogLines([]);
      const id = await trusted(server, (allow) => api.startLogStream(server.id, source, allow));
      setStreamId(id);
      setPaused(false);
      setLogStatus("实时流式日志已连接");
    } catch (reason) {
      setLogError(errorMessage(reason));
    } finally {
      setStartingLog(false);
    }
  };

  const stopStream = async () => {
    if (!streamId) return;
    await api.stopLogStream(streamId);
    setStreamId(null);
    setLogStatus("已停止日志流");
  };

  const loadProcesses = useCallback(async () => {
    setProcLoading(true);
    setProcError("");
    try {
      const list = await trusted(server, (allow) => api.listRemoteProcesses(server.id, 80, allow));
      setProcesses(list);
    } catch (reason) {
      setProcError(errorMessage(reason));
    } finally {
      setProcLoading(false);
    }
  }, [server, trusted]);

  useEffect(() => {
    if (subTab === "processes" && processes.length === 0 && !procLoading) {
      void loadProcesses();
    }
  }, [subTab, processes.length, procLoading, loadProcesses]);

  useEffect(() => {
    if (subTab !== "processes" || !autoRefresh) return;
    const timer = window.setInterval(() => {
      void loadProcesses();
    }, 5000);
    return () => window.clearInterval(timer);
  }, [subTab, autoRefresh, loadProcesses]);

  const sendSignal = async (proc: ProcessEntry, signal: "TERM" | "HUP" | "KILL") => {
    const label = signal === "KILL" ? "强制终止 (SIGKILL)" : signal === "HUP" ? "重载 (SIGHUP)" : "优雅终止 (SIGTERM)";
    if (!window.confirm(`确认向进程 PID ${proc.pid} (${proc.command.slice(0, 60)}) 发送 ${label}？`)) return;
    setSignalingPid(proc.pid);
    setProcError("");
    setProcNotice("");
    try {
      const msg = await trusted(server, (allow) => api.signalRemoteProcess(server.id, proc.pid, signal, allow));
      setProcNotice(msg);
      await loadProcesses();
    } catch (reason) {
      setProcError(errorMessage(reason));
    } finally {
      setSignalingPid(null);
    }
  };

  const filteredLogs = useMemo(() => {
    const q = logFilter.trim().toLowerCase();
    if (!q) return logLines;
    return logLines.filter((line) => line.toLowerCase().includes(q));
  }, [logLines, logFilter]);

  const filteredProcesses = useMemo(() => {
    const q = procFilter.trim().toLowerCase();
    const list = q
      ? processes.filter(
          (p) =>
            String(p.pid).includes(q) ||
            p.user.toLowerCase().includes(q) ||
            p.command.toLowerCase().includes(q),
        )
      : [...processes];
    list.sort((a, b) => {
      if (sortBy === "cpu") return b.cpuPercent - a.cpuPercent;
      if (sortBy === "mem") return b.memPercent - a.memPercent;
      return a.pid - b.pid;
    });
    return list;
  }, [processes, procFilter, sortBy]);

  return (
    <div className="panel-body">
      <div className="flex flex-wrap items-center justify-between gap-3 mb-4">
        <div className="row-actions">
          <button
            className={subTab === "logs" ? "btn btn-primary" : "btn"}
            type="button"
            onClick={() => setSubTab("logs")}
          >
            <ScrollText size={15} />
            实时日志流
          </button>
          <button
            className={subTab === "processes" ? "btn btn-primary" : "btn"}
            type="button"
            onClick={() => setSubTab("processes")}
          >
            <Activity size={15} />
            进程管理
          </button>
        </div>
        {subTab === "logs" && streamId && <span className="badge badge-ok">实时流运行中</span>}
      </div>

      {subTab === "logs" && (
        <div>
          <div className="flex flex-wrap items-end gap-3 mb-3">
            <div className="field">
              <label htmlFor="log-mode">日志来源</label>
              <select
                id="log-mode"
                className="select"
                value={logMode}
                disabled={Boolean(streamId)}
                onChange={(e) => setLogMode(e.target.value as LogMode)}
              >
                <option value="systemd">systemd (journalctl -f)</option>
                <option value="docker">Docker 容器 (docker logs -f)</option>
                <option value="file">指定文件 (tail -F)</option>
              </select>
            </div>
            {logMode === "systemd" && (
              <div className="field" style={{ minWidth: 200 }}>
                <label htmlFor="log-unit">服务名（留空看全部）</label>
                <input
                  id="log-unit"
                  className="input"
                  value={unit}
                  disabled={Boolean(streamId)}
                  onChange={(e) => setUnit(e.target.value)}
                  placeholder="例如 nginx / sshd"
                />
              </div>
            )}
            {logMode === "docker" && (
              <div className="field" style={{ minWidth: 200 }}>
                <label htmlFor="log-container">容器名或 ID</label>
                <input
                  id="log-container"
                  className="input"
                  value={container}
                  disabled={Boolean(streamId)}
                  onChange={(e) => setContainer(e.target.value)}
                  placeholder="例如 caddy / postgres"
                />
              </div>
            )}
            {logMode === "file" && (
              <div className="field" style={{ minWidth: 240 }}>
                <label htmlFor="log-file">文件路径</label>
                <input
                  id="log-file"
                  className="input"
                  value={filePath}
                  disabled={Boolean(streamId)}
                  onChange={(e) => setFilePath(e.target.value)}
                  placeholder="/var/log/nginx/error.log"
                />
              </div>
            )}
            <div className="field" style={{ width: 96 }}>
              <label htmlFor="log-lines">初始行数</label>
              <input
                id="log-lines"
                className="input"
                type="number"
                min={10}
                max={1000}
                value={initialLines}
                disabled={Boolean(streamId)}
                onChange={(e) => setInitialLines(Number(e.target.value) || 100)}
              />
            </div>
            <div className="row-actions">
              {!streamId ? (
                <button className="btn btn-primary" type="button" disabled={startingLog} onClick={() => void startStream()}>
                  <Play size={15} />
                  {startingLog ? "正在连接" : "开始实时流"}
                </button>
              ) : (
                <button className="btn btn-danger" type="button" onClick={() => void stopStream()}>
                  <Square size={15} />
                  停止流
                </button>
              )}
              <button className="btn" type="button" onClick={() => setPaused((v) => !v)}>
                {paused ? <Play size={15} /> : <Pause size={15} />}
                {paused ? "恢复滚动" : "暂停滚动"}
              </button>
              <button className="btn" type="button" onClick={() => setLogLines([])}>
                <Trash2 size={15} />
                清空
              </button>
            </div>
          </div>

          <div className="flex flex-wrap items-center justify-between gap-3 mb-3">
            <label className="search-field" style={{ minWidth: 240 }}>
              <Search size={14} aria-hidden="true" />
              <input
                className="input"
                value={logFilter}
                onChange={(e) => setLogFilter(e.target.value)}
                placeholder="实时过滤关键字（如 error / 502 / timeout）"
              />
            </label>
            <span className="table-meta">
              显示 {filteredLogs.length} / {logLines.length} 行{logStatus ? ` · ${logStatus}` : ""}
            </span>
          </div>

          {logError && <div className="error-banner mb-3" role="alert">{logError}</div>}

          <pre
            ref={logBoxRef}
            className="action-output"
            style={{ minHeight: 360, maxHeight: 520, overflowY: "auto", whiteSpace: "pre-wrap", wordBreak: "break-word" }}
          >
            <code>{filteredLogs.length > 0 ? filteredLogs.join("\n") : "选择日志来源并点击「开始实时流」开始实时追踪（纯 SSH，无需 Agent）。"}</code>
          </pre>
        </div>
      )}

      {subTab === "processes" && (
        <div>
          <div className="flex flex-wrap items-center justify-between gap-3 mb-3">
            <div className="row-actions">
              <label className="search-field" style={{ minWidth: 220 }}>
                <Search size={14} aria-hidden="true" />
                <input
                  className="input"
                  value={procFilter}
                  onChange={(e) => setProcFilter(e.target.value)}
                  placeholder="过滤 PID / 用户 / 命令"
                />
              </label>
              <select className="select" aria-label="排序方式" value={sortBy} onChange={(e) => setSortBy(e.target.value as SortBy)}>
                <option value="cpu">按 CPU 降序</option>
                <option value="mem">按内存降序</option>
                <option value="pid">按 PID 升序</option>
              </select>
            </div>
            <div className="row-actions">
              <label className="switch-control">
                <input type="checkbox" checked={autoRefresh} onChange={(e) => setAutoRefresh(e.target.checked)} />
                <span>每 5 秒刷新</span>
              </label>
              <button className="btn" type="button" disabled={procLoading} onClick={() => void loadProcesses()}>
                <RefreshCw size={15} />
                {procLoading ? "采集进程中" : "刷新"}
              </button>
            </div>
          </div>

          {procError && <div className="error-banner mb-3" role="alert">{procError}</div>}
          {procNotice && <div className="file-notice mb-3" role="status">{procNotice}</div>}

          <div style={{ overflowX: "auto" }}>
            <table className="data-table">
              <thead>
                <tr>
                  <th>PID</th>
                  <th>用户</th>
                  <th>CPU</th>
                  <th>内存</th>
                  <th>RSS</th>
                  <th>运行时间</th>
                  <th>命令</th>
                  <th>操作</th>
                </tr>
              </thead>
              <tbody>
                {filteredProcesses.map((proc) => (
                  <tr key={proc.pid}>
                    <td className="measured">{proc.pid}</td>
                    <td>{proc.user}</td>
                    <td className="measured">{proc.cpuPercent.toFixed(1)}%</td>
                    <td className="measured">{proc.memPercent.toFixed(1)}%</td>
                    <td className="measured">{formatBytes(proc.rssKb * 1024)}</td>
                    <td className="measured">{proc.elapsed}</td>
                    <td className="measured" style={{ maxWidth: 340, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }} title={proc.command}>
                      {proc.command}
                    </td>
                    <td>
                      <div className="row-actions">
                        <button
                          className="btn"
                          type="button"
                          disabled={signalingPid === proc.pid || proc.pid <= 1}
                          onClick={() => void sendSignal(proc, "HUP")}
                          title="发送 SIGHUP（常用于重载配置）"
                        >
                          HUP
                        </button>
                        <button
                          className="btn"
                          type="button"
                          disabled={signalingPid === proc.pid || proc.pid <= 1}
                          onClick={() => void sendSignal(proc, "TERM")}
                          title="发送 SIGTERM（优雅终止）"
                        >
                          TERM
                        </button>
                        <button
                          className="btn btn-danger"
                          type="button"
                          disabled={signalingPid === proc.pid || proc.pid <= 1}
                          onClick={() => void sendSignal(proc, "KILL")}
                          title="发送 SIGKILL（强制终止）"
                        >
                          KILL
                        </button>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            {filteredProcesses.length === 0 && !procLoading && (
              <p className="table-meta mt-4">暂无匹配的进程记录。</p>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
