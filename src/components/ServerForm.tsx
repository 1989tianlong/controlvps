import { useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import type { AuthMethod, Server, ServerInput } from "../types";

const emptyForm: ServerInput = {
  name: "",
  host: "",
  port: 22,
  username: "root",
  authMethod: "key",
  privateKeyPath: "~/.ssh/id_ed25519",
  tags: [],
  currency: "USD",
};

interface Props {
  server: Server | null;
  servers: Server[];
  open: boolean;
  onClose: () => void;
  onSaved: (server: Server) => void;
}

export function ServerForm({ server, servers, open, onClose, onSaved }: Props) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [form, setForm] = useState<ServerInput>(emptyForm);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!open) return;
    setError("");
    setForm(server ? {
      name: server.name,
      host: server.host,
      port: server.port,
      username: server.username,
      authMethod: server.authMethod,
      privateKeyPath: server.privateKeyPath ?? "",
      groupName: server.groupName ?? "",
      tags: server.tags,
      provider: server.provider ?? "",
      region: server.region ?? "",
      notes: server.notes ?? "",
      cost: server.cost,
      currency: server.currency ?? "USD",
      expiresAt: server.expiresAt ?? "",
      jumpServerId: server.jumpServerId ?? "",
    } : emptyForm);
    dialogRef.current?.showModal();
  }, [open, server]);

  const update = <K extends keyof ServerInput>(key: K, value: ServerInput[K]) => {
    setForm((current) => ({ ...current, [key]: value }));
  };

  const close = () => {
    if (!busy) {
      dialogRef.current?.close();
      onClose();
    }
  };

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError("");
    try {
      const saved = server ? await api.updateServer(server.id, form) : await api.createServer(form);
      onSaved(saved);
      dialogRef.current?.close();
      onClose();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  if (!open) return null;

  return (
    <dialog ref={dialogRef} className="dialog" onCancel={(event) => { event.preventDefault(); close(); }}>
      <form onSubmit={submit}>
        <div className="dialog-header">
          <h2>{server ? "编辑 VPS" : "添加 VPS"}</h2>
          <button className="icon-btn" type="button" aria-label="关闭" onClick={close}><X size={18} /></button>
        </div>
        <div className="dialog-body">
          {error && <div className="error-banner" role="alert">{error}</div>}
          <section className="form-section">
            <h2>服务器</h2>
            <div className="form-grid">
              <div className="field">
                <label htmlFor="server-name">名称，必填</label>
                <input className="input" id="server-name" name="name" autoComplete="off" required value={form.name} onChange={(e) => update("name", e.target.value)} placeholder="生产网站" />
              </div>
              <div className="field">
                <label htmlFor="server-group">分组</label>
                <input className="input" id="server-group" name="group" autoComplete="off" value={form.groupName ?? ""} onChange={(e) => update("groupName", e.target.value)} placeholder="生产环境" />
              </div>
              <div className="field span-2">
                <label htmlFor="server-host">IP 或域名，必填</label>
                <input className="input" id="server-host" name="host" autoComplete="off" required value={form.host} onChange={(e) => update("host", e.target.value)} placeholder="203.0.113.10" />
              </div>
              <div className="field">
                <label htmlFor="server-port">SSH 端口</label>
                <input className="input" id="server-port" name="port" type="number" inputMode="numeric" min={1} max={65535} required value={form.port} onChange={(e) => update("port", Number(e.target.value))} />
              </div>
              <div className="field">
                <label htmlFor="server-user">用户名，必填</label>
                <input className="input" id="server-user" name="username" autoComplete="off" required value={form.username} onChange={(e) => update("username", e.target.value)} />
              </div>
            </div>
          </section>
          <section className="form-section">
            <h2>连接路由</h2>
            <div className="field">
              <label htmlFor="jump-server">跳板机</label>
              <select className="select w-full" id="jump-server" name="jumpServerId" value={form.jumpServerId ?? ""} onChange={(e) => update("jumpServerId", e.target.value || undefined)}>
                <option value="">直接连接</option>
                {servers.filter((item) => item.id !== server?.id && !item.jumpServerId).map((item) => <option key={item.id} value={item.id}>{item.name} · {item.host}:{item.port}</option>)}
              </select>
              <p className="field-help">先通过本机连接跳板机，再由跳板机访问目标 SSH 端口。当前支持一级跳板，两台服务器分别校验主机密钥并使用各自凭据。</p>
            </div>
          </section>
          <section className="form-section">
            <h2>认证</h2>
            <div className="form-grid">
              <div className="field span-2">
                <label htmlFor="auth-method">认证方式</label>
                <select className="select w-full" id="auth-method" name="authMethod" value={form.authMethod} onChange={(e) => update("authMethod", e.target.value as AuthMethod)}>
                  <option value="key">SSH 私钥</option>
                  <option value="password">密码</option>
                  <option value="agent">SSH Agent</option>
                </select>
              </div>
              {form.authMethod === "key" && <>
                <div className="field span-2">
                  <label htmlFor="key-path">私钥路径，必填</label>
                  <input className="input" id="key-path" name="privateKeyPath" required value={form.privateKeyPath ?? ""} onChange={(e) => update("privateKeyPath", e.target.value)} />
                  <p className="field-help">只读取本机文件，私钥内容不会复制到数据库。</p>
                </div>
                <div className="field span-2">
                  <label htmlFor="key-passphrase">私钥密码{server?.hasKeyPassphrase ? "，留空表示保留" : ""}</label>
                  <input className="input" id="key-passphrase" name="keyPassphrase" type="password" autoComplete="new-password" value={form.keyPassphrase ?? ""} onChange={(e) => update("keyPassphrase", e.target.value)} />
                </div>
              </>}
              {form.authMethod === "password" && <div className="field span-2">
                <label htmlFor="server-password">SSH 密码{server ? "，留空表示保留" : "，必填"}</label>
                <input className="input" id="server-password" name="password" type="password" autoComplete="new-password" required={!server} value={form.password ?? ""} onChange={(e) => update("password", e.target.value)} />
                <p className="field-help">密码保存在 macOS Keychain，不会写入 SQLite。</p>
              </div>}
              {form.authMethod === "agent" && <p className="text-muted span-2">使用当前系统 SSH Agent 中已加载的身份，请确认 `ssh-add -L` 能列出密钥。</p>}
            </div>
          </section>
          <section className="form-section">
            <h2>资产信息</h2>
            <div className="form-grid">
              <div className="field"><label htmlFor="provider">服务商</label><input className="input" id="provider" name="provider" value={form.provider ?? ""} onChange={(e) => update("provider", e.target.value)} /></div>
              <div className="field"><label htmlFor="region">地区</label><input className="input" id="region" name="region" value={form.region ?? ""} onChange={(e) => update("region", e.target.value)} placeholder="新加坡" /></div>
              <div className="field"><label htmlFor="cost">每月成本</label><input className="input" id="cost" name="cost" type="number" min={0} step="0.01" value={form.cost ?? ""} onChange={(e) => update("cost", e.target.value ? Number(e.target.value) : undefined)} /></div>
              <div className="field"><label htmlFor="currency">货币</label><select className="select w-full" id="currency" name="currency" value={form.currency} onChange={(e) => update("currency", e.target.value)}><option value="USD">USD</option><option value="EUR">EUR</option><option value="CNY">CNY</option></select></div>
              <div className="field"><label htmlFor="expires">到期日期</label><input className="input" id="expires" name="expiresAt" type="date" value={form.expiresAt ?? ""} onChange={(e) => update("expiresAt", e.target.value)} /></div>
              <div className="field"><label htmlFor="tags">标签，用逗号分隔</label><input className="input" id="tags" name="tags" value={form.tags.join(", ")} onChange={(e) => update("tags", e.target.value.split(/[,，]/).map((tag) => tag.trim()).filter(Boolean))} /></div>
              <div className="field span-2"><label htmlFor="notes">备注</label><textarea className="textarea" id="notes" name="notes" value={form.notes ?? ""} onChange={(e) => update("notes", e.target.value)} /></div>
            </div>
          </section>
        </div>
        <div className="dialog-footer">
          <button className="btn" type="button" onClick={close} disabled={busy}>取消</button>
          <button className="btn btn-primary" type="submit" disabled={busy} aria-busy={busy}>{busy ? "正在保存" : server ? "保存修改" : "添加服务器"}</button>
        </div>
      </form>
    </dialog>
  );
}
