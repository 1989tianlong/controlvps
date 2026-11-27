import { useCallback, useEffect, useRef, useState } from "react";
import { Clapperboard, Pause, Play, RefreshCw, RotateCcw, ShieldAlert, Trash2, X } from "lucide-react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import { api, errorMessage } from "../lib/api";
import type { RecordingDetail, RecordingSummary } from "../types";
import "@xterm/xterm/css/xterm.css";

function durationText(seconds: number) {
  if (!seconds || seconds <= 0) return "—";
  const total = Math.round(seconds);
  const minutes = Math.floor(total / 60);
  const rest = total % 60;
  return `${minutes}:${String(rest).padStart(2, "0")}`;
}

function sizeText(bytes: number) {
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${bytes} B`;
}

function dateText(value: string) {
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value || "未知时间";
  return parsed.toLocaleString("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" });
}

const speeds = [0.5, 1, 2, 4];

export function RecordingsPage() {
  const [items, setItems] = useState<RecordingSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState("");
  const [playing, setPlaying] = useState<RecordingDetail | null>(null);

  const load = useCallback(async () => {
    setError("");
    try {
      setItems(await api.listTerminalRecordings());
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const play = async (item: RecordingSummary) => {
    setBusy(item.id);
    setError("");
    try {
      setPlaying(await api.readTerminalRecording(item.id));
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy("");
    }
  };

  const remove = async (item: RecordingSummary) => {
    if (!window.confirm(`删除 ${item.serverName} 在 ${dateText(item.startedAt)} 的这段录制？文件会从本机移除。`)) return;
    setBusy(item.id);
    setError("");
    try {
      await api.deleteTerminalRecording(item.id);
      if (playing?.summary.id === item.id) setPlaying(null);
      await load();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy("");
    }
  };

  return <>
    <div className="page-heading"><div><h1>录制回放</h1><p>终端会话可录制为 asciicast v2 文件，保存在本机，用于复盘操作和排查事故。</p></div><button className="btn" type="button" onClick={() => void load()} disabled={loading}><RefreshCw size={15} className={loading ? "spin" : undefined} />刷新</button></div>
    <div className="risk flex items-start gap-3"><ShieldAlert size={20} className="shrink-0" /><p>录制仅保存终端输出（新录制不再写入原始键盘输入，但屏幕上回显的命令与输出仍可能含令牌等敏感内容）；文件以 0600 权限保存在本机应用数据目录，不会随加密备份导出，可随时在这里删除。</p></div>
    {error && <div className="error-banner" role="alert"><p>{error}</p></div>}
    {playing ? <PlaybackPlayer detail={playing} onClose={() => setPlaying(null)} /> : loading ? <div className="skeleton recording-skeleton" /> : items.length === 0 ? <div className="empty"><div><Clapperboard size={32} /><h2>还没有录制</h2><p>在「终端工作区」打开会话后，点顶部的录制按钮开始录制，停止后就会出现在这里。</p></div></div> : <div className="table-wrap panel"><table className="data-table recording-table"><thead><tr><th>服务器</th><th>开始时间</th><th>时长</th><th>大小</th><th>帧数</th><th>操作</th></tr></thead><tbody>{items.map((item) => <tr key={item.id}>
      <td><span className="table-name">{item.serverName}</span><span className="table-meta">{item.width}×{item.height}</span></td>
      <td className="measured">{dateText(item.startedAt)}</td>
      <td className="measured">{durationText(item.durationSeconds)}</td>
      <td className="measured">{sizeText(item.sizeBytes)}</td>
      <td className="measured">{item.frames}</td>
      <td><div className="row-actions">
        <button className="btn" type="button" aria-busy={busy === item.id} disabled={busy === item.id} onClick={() => void play(item)}><Play size={14} />回放</button>
        <button className="icon-btn" type="button" aria-label={`删除 ${item.serverName} 的录制`} onClick={() => void remove(item)}><Trash2 size={16} /></button>
      </div></td>
    </tr>)}</tbody></table></div>}
  </>;
}

function PlaybackPlayer({ detail, onClose }: { detail: RecordingDetail; onClose: () => void }) {
  const hostRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const indexRef = useRef(0);
  const clockRef = useRef(0);
  const [running, setRunning] = useState(true);
  const [speed, setSpeed] = useState(1);
  const [progress, setProgress] = useState(0);
  const [restartKey, setRestartKey] = useState(0);
  const total = detail.summary.durationSeconds || detail.frames.reduce((max, frame) => Math.max(max, frame.time), 0);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const css = getComputedStyle(document.documentElement);
    const token = (name: string) => css.getPropertyValue(name).trim();
    const terminal = new Terminal({
      disableStdin: true,
      cursorBlink: false,
      fontFamily: token("--font-outlier"),
      fontSize: 13,
      lineHeight: 1.35,
      scrollback: 3000,
      theme: {
        background: token("--color-paper-3"),
        foreground: token("--color-ink"),
        cursor: token("--color-accent"),
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
    terminal.open(host);
    const hasRecordedDims = detail.summary.width > 0 && detail.summary.height > 0 && detail.summary.width <= 500;
    if (hasRecordedDims) {
      terminal.resize(detail.summary.width, detail.summary.height);
    } else {
      fit.fit();
    }
    terminalRef.current = terminal;
    indexRef.current = 0;
    clockRef.current = 0;
    const observer = new ResizeObserver(() => {
      if (!hasRecordedDims) fit.fit();
    });
    observer.observe(host);
    return () => {
      observer.disconnect();
      terminal.dispose();
      terminalRef.current = null;
    };
  }, [detail, restartKey]);

  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal || !running) return;
    const frames = detail.frames.filter((frame) => frame.kind !== "i");
    let index = indexRef.current;
    let clock = clockRef.current;
    let timer = 0;
    let cancelled = false;
    const step = () => {
      if (cancelled) return;
      if (index >= frames.length) {
        setRunning(false);
        setProgress(1);
        return;
      }
      const frame = frames[index];
      const delay = Math.max(0, ((frame.time - clock) * 1000) / speed);
      timer = window.setTimeout(() => {
        if (cancelled) return;
        if (frame.kind === "r") {
          const [columns, rows] = frame.data.split("x").map(Number);
          if (columns > 0 && rows > 0 && columns <= 500) terminal.resize(columns, rows);
        } else {
          terminal.write(frame.data);
        }
        clock = frame.time;
        index += 1;
        indexRef.current = index;
        clockRef.current = clock;
        setProgress(total > 0 ? Math.min(1, clock / total) : 0);
        step();
      }, delay);
    };
    step();
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [running, speed, detail, total, restartKey]);

  const restart = () => {
    indexRef.current = 0;
    clockRef.current = 0;
    setProgress(0);
    setRunning(true);
    setRestartKey((value) => value + 1);
  };

  const skippedInputs = detail.frames.filter((frame) => frame.kind === "i").length;
  return <section className="panel playback-panel">
    <div className="panel-header">
      <div><span className="table-name">{detail.summary.serverName}</span><span className="table-meta">{dateText(detail.summary.startedAt)} · {durationText(detail.summary.durationSeconds)} · {detail.frames.length} 帧{skippedInputs > 0 ? `（含 ${skippedInputs} 段键盘输入）` : ""}</span></div>
      <div className="row-actions">
        <button className="icon-btn" type="button" aria-label={running ? "暂停" : "继续"} title={running ? "暂停" : "继续"} onClick={() => setRunning((value) => !value)}>{running ? <Pause size={16} /> : <Play size={16} />}</button>
        <button className="icon-btn" type="button" aria-label="从头播放" title="从头播放" onClick={restart}><RotateCcw size={16} /></button>
        <select className="select" aria-label="播放速度" value={speed} onChange={(event) => setSpeed(Number(event.target.value))}>{speeds.map((value) => <option key={value} value={value}>{value}×</option>)}</select>
        <button className="icon-btn" type="button" aria-label="关闭回放" title="关闭回放" onClick={onClose}><X size={16} /></button>
      </div>
    </div>
    <div ref={hostRef} className="terminal-shell playback-shell" aria-label={`${detail.summary.serverName} 录制回放`} />
    <div className="playback-foot">
      <progress className="playback-progress" max={1} value={progress} aria-label="回放进度" />
      <span className="measured">{Math.round(progress * 100)}%</span>
      <span className="text-muted">回放只显示终端输出（新录制不再记录原始键盘输入）。</span>
    </div>
  </section>;
}
