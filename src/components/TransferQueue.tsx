import { useCallback, useEffect, useRef, useState } from "react";
import { ArrowDownToLine, ArrowUpFromLine, RotateCcw, Trash2, X } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import { formatBytes } from "../lib/format";
import type { FileTransferTask } from "../types";

interface Props {
  serverId: string;
  revision: number;
  onCompleted: () => void;
}

const statusLabels: Record<FileTransferTask["status"], string> = {
  queued: "排队中",
  running: "传输中",
  completed: "已完成",
  failed: "失败",
  cancelled: "已取消",
};

export function TransferQueue({ serverId, revision, onCompleted }: Props) {
  const [tasks, setTasks] = useState<FileTransferTask[]>([]);
  const [error, setError] = useState("");
  const previousActive = useRef(new Set<string>());
  const completedCallback = useRef(onCompleted);
  completedCallback.current = onCompleted;

  const load = useCallback(async () => {
    try {
      const next = (await api.listFileTransfers()).filter((task) => task.serverId === serverId);
      const completed = next.some((task) => task.status === "completed" && previousActive.current.has(task.id));
      previousActive.current = new Set(next.filter((task) => task.status === "queued" || task.status === "running").map((task) => task.id));
      setTasks(next);
      setError("");
      if (completed) completedCallback.current();
    } catch (reason) {
      setError(errorMessage(reason));
    }
  }, [serverId]);

  useEffect(() => {
    previousActive.current = new Set();
    void load();
  }, [load, revision]);

  const active = tasks.some((task) => task.status === "queued" || task.status === "running");
  useEffect(() => {
    if (!active) return;
    const timer = window.setInterval(() => void load(), 800);
    return () => window.clearInterval(timer);
  }, [active, load]);

  const run = async (operation: () => Promise<unknown>) => {
    setError("");
    try {
      await operation();
      await load();
    } catch (reason) {
      setError(errorMessage(reason));
    }
  };

  const finished = tasks.some((task) => task.status === "completed" || task.status === "failed" || task.status === "cancelled");
  return <section className="transfer-queue" aria-label="文件传输队列">
    <div className="transfer-heading">
      <div><h3>传输队列</h3><p>最多同时传输 2 个文件；上传和下载完成后才替换目标文件。</p></div>
      {finished && <button className="btn btn-small" type="button" onClick={() => void run(() => api.clearFinishedFileTransfers(serverId))}><Trash2 size={14} />清理已结束</button>}
    </div>
    {error && <div className="error-banner" role="alert">{error}</div>}
    {tasks.length === 0 ? <p className="transfer-empty">暂无传输任务。可选择多个文件批量上传。</p> : <div className="transfer-list">{tasks.map((task) => {
      const percent = task.totalBytes > 0 ? Math.min(100, task.transferredBytes / task.totalBytes * 100) : 0;
      const isActive = task.status === "queued" || task.status === "running";
      return <article className="transfer-item" key={task.id}>
        <div className="transfer-icon">{task.kind === "upload" ? <ArrowUpFromLine size={17} /> : <ArrowDownToLine size={17} />}</div>
        <div className="transfer-content">
          <div className="transfer-title"><strong>{task.fileName}</strong><span className={`badge ${task.status === "completed" ? "badge-ok" : task.status === "failed" ? "badge-bad" : ""}`}>{statusLabels[task.status]}</span></div>
          <div className="transfer-meta"><span>{task.kind === "upload" ? "上传到" : "下载自"} {task.remotePath}</span><span>{formatBytes(task.transferredBytes)} / {task.totalBytes ? formatBytes(task.totalBytes) : "未知"}{task.status === "running" && task.bytesPerSecond > 0 ? ` · ${formatBytes(task.bytesPerSecond)}/s` : ""}</span></div>
          {(task.status === "running" || task.status === "queued") && <div className="transfer-progress" role="progressbar" aria-label={`${task.fileName} 传输进度`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(percent)}><span style={{ width: `${task.status === "queued" ? 0 : percent}%` }} /></div>}
          {task.error && <p className="transfer-error">{task.error}</p>}
        </div>
        <div className="row-actions">
          {isActive && <button className="icon-btn" type="button" title="取消传输" aria-label={`取消 ${task.fileName}`} onClick={() => void run(() => api.cancelFileTransfer(task.id))}><X size={16} /></button>}
          {(task.status === "failed" || task.status === "cancelled") && <button className="icon-btn" type="button" title="重新传输" aria-label={`重试 ${task.fileName}`} onClick={() => void run(() => api.retryFileTransfer(task.id))}><RotateCcw size={16} /></button>}
        </div>
      </article>;
    })}</div>}
  </section>;
}
