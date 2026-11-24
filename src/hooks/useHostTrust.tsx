import { useCallback, useEffect, useRef, useState } from "react";
import { ShieldAlert, X } from "lucide-react";
import { CommandError } from "../lib/api";
import type { Server } from "../types";

interface Challenge {
  server: Server;
  fingerprint: string;
  host: string;
  operation: (allow: boolean) => Promise<unknown>;
  resolve: (value: unknown) => void;
  reject: (error: unknown) => void;
}

export function useHostTrust() {
  const [challenge, setChallenge] = useState<Challenge | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const dialogRef = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    if (challenge && !dialogRef.current?.open) dialogRef.current?.showModal();
    if (!challenge && dialogRef.current?.open) dialogRef.current.close();
  }, [challenge]);

  const execute = useCallback(<T,>(server: Server, operation: (allow: boolean) => Promise<T>): Promise<T> => {
    return new Promise<T>((resolve, reject) => {
      operation(false)
        .then(resolve)
        .catch((reason: unknown) => {
          if (reason instanceof CommandError && reason.code === "HOST_KEY_UNKNOWN") {
            setError("");
            setChallenge({
              server,
              fingerprint: reason.fingerprint ?? "指纹不可用",
              host: reason.host ?? `${server.host}:${server.port}`,
              operation,
              resolve: resolve as (value: unknown) => void,
              reject,
            });
          } else {
            reject(reason);
          }
        });
    });
  }, []);

  const cancel = () => {
    challenge?.reject(new CommandError({ code: "CANCELLED", message: "已取消连接" }));
    setChallenge(null);
  };

  const confirm = async () => {
    if (!challenge) return;
    setBusy(true);
    setError("");
    try {
      const result = await challenge.operation(true);
      challenge.resolve(result);
      setChallenge(null);
    } catch (reason) {
      if (reason instanceof CommandError && reason.code === "HOST_KEY_UNKNOWN") {
        setChallenge({
          ...challenge,
          fingerprint: reason.fingerprint ?? "指纹不可用",
          host: reason.host ?? challenge.host,
        });
      } else {
        challenge.reject(reason);
        setError(reason instanceof Error ? reason.message : String(reason));
        setChallenge(null);
      }
    } finally {
      setBusy(false);
    }
  };

  const trustDialog = (
    <dialog ref={dialogRef} className="dialog" onCancel={(event) => { event.preventDefault(); cancel(); }}>
      <div className="dialog-header">
        <h2>确认服务器身份</h2>
        <button className="icon-btn" type="button" aria-label="关闭" onClick={cancel}><X size={18} /></button>
      </div>
      <div className="dialog-body">
        <ShieldAlert size={28} color="var(--color-accent)" aria-hidden="true" />
        <p className="mt-3">正在连接 <strong>{challenge?.host ?? challenge?.server.name}</strong>。请通过服务商控制台或其他可信渠道核对这一跳的 SSH 主机密钥指纹。</p>
        <code className="fingerprint">{challenge?.fingerprint}</code>
        <p className="text-muted text-sm">确认后指纹会写入本机 known_hosts。后续指纹变化时 ControlVPS 会拒绝连接。</p>
        {error && <p className="error-banner" role="alert">{error}</p>}
      </div>
      <div className="dialog-footer">
        <button className="btn" type="button" onClick={cancel} disabled={busy}>取消</button>
        <button className="btn btn-primary" type="button" onClick={confirm} disabled={busy} aria-busy={busy}>
          {busy ? "正在连接" : "信任并连接"}
        </button>
      </div>
    </dialog>
  );

  return { execute, trustDialog };
}
