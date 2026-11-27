import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Check, ShieldAlert, X } from "lucide-react";
import { api } from "../lib/api";
import type { McpApprovalRequest } from "../types";

export function McpApprovalDialog() {
  const [queue, setQueue] = useState<McpApprovalRequest[]>([]);
  const [remaining, setRemaining] = useState<number>(90);
  const [busy, setBusy] = useState(false);
  const dialogRef = useRef<HTMLDialogElement>(null);
  const active = queue[0] ?? null;

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void api.listPendingMcpApprovals()
      .then((items) => {
        if (!disposed && items.length > 0) setQueue(items);
      })
      .catch(() => undefined);
    void listen<McpApprovalRequest>("mcp-approval-request", (event) => {
      const req = event.payload;
      if (!req?.id) return;
      setQueue((current) => current.some((item) => item.id === req.id) ? current : [...current, req]);
    }).then((dispose) => {
      if (disposed) dispose();
      else unlisten = dispose;
    }).catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (active && !dialogRef.current?.open) dialogRef.current?.showModal();
    if (!active && dialogRef.current?.open) dialogRef.current.close();
  }, [active]);

  useEffect(() => {
    if (!active) return;
    const deadline = Date.parse(active.createdAt) + active.timeoutSeconds * 1000;
    const update = () => {
      const secs = Math.max(0, Math.ceil((deadline - Date.now()) / 1000));
      setRemaining(secs);
      if (secs === 0) {
        setQueue((current) => current.filter((item) => item.id !== active.id));
      }
    };
    update();
    const timer = window.setInterval(update, 1000);
    return () => window.clearInterval(timer);
  }, [active]);

  const decide = async (approved: boolean) => {
    if (!active || busy) return;
    setBusy(true);
    try {
      await api.resolveMcpApproval(active.id, approved);
    } catch {
      // ignore if already timed out
    } finally {
      setQueue((current) => current.filter((item) => item.id !== active.id));
      setBusy(false);
    }
  };

  return (
    <dialog
      ref={dialogRef}
      className="dialog"
      onCancel={(event) => {
        event.preventDefault();
        void decide(false);
      }}
    >
      {active && <>
        <div className="dialog-header">
          <div>
            <h2>MCP 写操作审批请求</h2>
            <span className="table-meta">外部 AI 客户端请求执行写操作 · 剩余 {remaining} 秒{queue.length > 1 ? `（还有 ${queue.length - 1} 条排队）` : ""}</span>
          </div>
          <button className="icon-btn" type="button" aria-label="拒绝并关闭" onClick={() => void decide(false)} disabled={busy}>
            <X size={18} />
          </button>
        </div>
        <div className="dialog-body">
          {active.dangerous && (
            <div className="risk flex items-start gap-3 mb-4" role="alert">
              <ShieldAlert size={20} className="shrink-0" />
              <p><strong>检测到高风险破坏性命令模式</strong>（如递归删除、重启/关机、磁盘写入或防火墙清空）。请逐字核对后再决定是否放行。</p>
            </div>
          )}
          <dl className="facts mb-4">
            <div><dt>请求工具</dt><dd><code>{active.tool}</code> · {active.title}</dd></div>
            <div><dt>影响目标</dt><dd><strong>{active.target || "未指定"}</strong></dd></div>
          </dl>
          <div className="field">
            <span className="field-label">待执行命令 / 步骤预览</span>
            <pre className="review-command"><code>{active.command}</code></pre>
          </div>
          <p className="field-help mt-3">批准仅对这一次调用生效，执行结果与命令摘要将记入操作审计日志（标记为 `[AI/MCP]`）。</p>
        </div>
        <div className="dialog-footer">
          <button className="btn" type="button" disabled={busy} onClick={() => void decide(false)}>拒绝</button>
          <button className={active.dangerous ? "btn btn-danger" : "btn btn-primary"} type="button" disabled={busy} onClick={() => void decide(true)}>
            <Check size={15} />批准本次执行
          </button>
        </div>
      </>}
    </dialog>
  );
}
