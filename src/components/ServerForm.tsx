import { useEffect, useRef, useState } from "react";
import { ArrowDown, ArrowUp, Plus, X } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import type { AuthMethod, AuthProfile, Server, ServerGroup, ServerInput } from "../types";

const emptyForm: ServerInput = {
  name: "",
  host: "",
  port: 22,
  username: "root",
  authMethod: "key",
  privateKeyPath: "~/.ssh/id_ed25519",
  tags: [],
  currency: "USD",
  jumpChain: [],
  inheritPort: false,
  inheritUsername: false,
  inheritJump: false,
  inheritAuth: false,
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
  const [groups, setGroups] = useState<ServerGroup[]>([]);
  const [profiles, setProfiles] = useState<AuthProfile[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [pendingHop, setPendingHop] = useState("");
  const selectedGroup = groups.find((group) => group.name === form.groupName);
  const inheritedProfile = profiles.find((profile) => profile.id === (form.inheritAuth ? selectedGroup?.authProfileId : form.authProfileId));

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
      jumpChain: server.jumpChain?.length ? server.jumpChain : server.jumpServerId ? [server.jumpServerId] : [],
      authProfileId: server.authProfileId ?? "",
      inheritPort: server.inheritPort,
      inheritUsername: server.inheritUsername,
      inheritJump: server.inheritJump,
      inheritAuth: server.inheritAuth,
    } : emptyForm);
    void Promise.all([api.serverGroups(), api.authProfiles()])
      .then(([nextGroups, nextProfiles]) => {
        setGroups(nextGroups);
        setProfiles(nextProfiles);
      })
      .catch((reason) => setError(errorMessage(reason)));
    dialogRef.current?.showModal();
  }, [open, server]);

  const update = <K extends keyof ServerInput>(key: K, value: ServerInput[K]) => {
    setForm((current) => ({ ...current, [key]: value }));
  };

  const addHop = () => {
    if (!pendingHop || form.jumpChain.includes(pendingHop) || form.jumpChain.length >= 4) return;
    update("jumpChain", [...form.jumpChain, pendingHop]);
    setPendingHop("");
  };

  const removeHop = (index: number) => {
    update("jumpChain", form.jumpChain.filter((_, position) => position !== index));
  };

  const moveHop = (index: number, offset: number) => {
    const target = index + offset;
    if (target < 0 || target >= form.jumpChain.length) return;
    const next = [...form.jumpChain];
    [next[index], next[target]] = [next[target], next[index]];
    update("jumpChain", next);
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
      const payload: ServerInput = {
        ...form,
        port: form.inheritPort ? 22 : form.port,
        username: form.inheritUsername ? "root" : form.username,
        jumpServerId: form.inheritJump ? undefined : form.jumpChain[0],
        jumpChain: form.inheritJump ? [] : form.jumpChain,
        authMethod: form.inheritAuth ? "key" : form.authMethod,
        privateKeyPath: form.inheritAuth ? "~/.ssh/id_ed25519" : form.privateKeyPath,
        authProfileId: form.inheritAuth ? undefined : form.authProfileId,
        password: form.inheritAuth ? undefined : form.password,
        keyPassphrase: form.inheritAuth ? undefined : form.keyPassphrase,
      };
      const saved = server ? await api.updateServer(server.id, payload) : await api.createServer(payload);
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
                <select className="select w-full" id="server-group" name="group" value={groups.some((group) => group.name === form.groupName) ? form.groupName : ""} onChange={(e) => update("groupName", e.target.value)}>
                  <option value="">不使用分组</option>
                  {groups.map((group) => <option key={group.id} value={group.name}>{group.name}</option>)}
                </select>
                <p className="field-help">{selectedGroup ? "连接时沿用分组里勾选的默认项；这台机器单独填写的值优先。" : "分组在「云资产」中创建。自由输入的旧分组名称不会带连接默认值。"}</p>
              </div>
              <div className="field span-2">
                <label htmlFor="server-host">IP 或域名，必填</label>
                <input className="input" id="server-host" name="host" autoComplete="off" required value={form.host} onChange={(e) => update("host", e.target.value)} placeholder="203.0.113.10" />
              </div>
              <div className="field">
                <label htmlFor="server-port">SSH 端口</label>
                <input className="input" id="server-port" name="port" type="number" inputMode="numeric" min={1} max={65535} required={!form.inheritPort} disabled={form.inheritPort} value={form.inheritPort ? selectedGroup?.defaultPort ?? "" : form.port} onChange={(e) => update("port", Number(e.target.value))} />
                {selectedGroup?.defaultPort && <label className="check-row"><input type="checkbox" checked={Boolean(form.inheritPort)} onChange={(e) => update("inheritPort", e.target.checked)} />沿用分组端口 {selectedGroup.defaultPort}</label>}
              </div>
              <div className="field">
                <label htmlFor="server-user">用户名{form.inheritUsername ? "" : "，必填"}</label>
                <input className="input" id="server-user" name="username" autoComplete="off" required={!form.inheritUsername} disabled={form.inheritUsername} value={form.inheritUsername ? selectedGroup?.defaultUsername ?? "" : form.username} onChange={(e) => update("username", e.target.value)} />
                {selectedGroup?.defaultUsername && <label className="check-row"><input type="checkbox" checked={Boolean(form.inheritUsername)} onChange={(e) => update("inheritUsername", e.target.checked)} />沿用分组用户名 {selectedGroup.defaultUsername}</label>}
              </div>
            </div>
          </section>
          <section className="form-section">
            <h2>连接路由</h2>
            <div className="field">
              <span className="field-label">跳板链</span>
              <ol className="jump-chain" aria-label="跳板链">
                {form.jumpChain.length === 0 ? <li className="jump-empty">直接连接，不经过跳板</li> : form.jumpChain.map((hopId, index) => {
                  const hop = servers.find((item) => item.id === hopId);
                  return <li key={`${hopId}-${index}`}>
                    <span className="jump-index" aria-hidden="true">{index + 1}</span>
                    <span className="jump-name">{hop ? <>{hop.name}<small>{hop.username}@{hop.host}:{hop.port}</small></> : "跳板机已删除"}</span>
                    <span className="jump-actions">
                      <button className="icon-btn" type="button" aria-label={`上移第 ${index + 1} 跳`} disabled={form.inheritJump || index === 0} onClick={() => moveHop(index, -1)}><ArrowUp size={15} /></button>
                      <button className="icon-btn" type="button" aria-label={`下移第 ${index + 1} 跳`} disabled={form.inheritJump || index === form.jumpChain.length - 1} onClick={() => moveHop(index, 1)}><ArrowDown size={15} /></button>
                      <button className="icon-btn" type="button" aria-label={`移除第 ${index + 1} 跳`} disabled={form.inheritJump} onClick={() => removeHop(index)}><X size={15} /></button>
                    </span>
                  </li>;
                })}
              </ol>
              <div className="jump-add">
                <select className="select" aria-label="选择要追加的跳板机" disabled={form.inheritJump || form.jumpChain.length >= 4} value={pendingHop} onChange={(e) => setPendingHop(e.target.value)}>
                  <option value="">选择要追加的跳板机</option>
                  {servers.filter((item) => item.id !== server?.id && !form.jumpChain.includes(item.id)).map((item) => <option key={item.id} value={item.id}>{item.name} · {item.host}:{item.port}</option>)}
                </select>
                <button className="btn" type="button" disabled={form.inheritJump || !pendingHop || form.jumpChain.length >= 4} onClick={addHop}><Plus size={15} />追加一跳</button>
              </div>
              {selectedGroup && <label className="check-row"><input type="checkbox" checked={Boolean(form.inheritJump)} onChange={(e) => update("inheritJump", e.target.checked)} />沿用分组默认跳板机{selectedGroup.jumpServerId ? "" : "（分组当前为直接连接）"}</label>}
              <p className="field-help">按顺序逐跳连接：本机先连第 1 跳，再由它连第 2 跳，最后到达目标 SSH 端口，最多 4 跳。每台服务器分别校验主机密钥并使用各自凭据；能作为跳板的前提是上一跳所在网络可以访问它的地址。</p>
            </div>
          </section>
          <section className="form-section">
            <h2>认证</h2>
            <div className="form-grid">
              {selectedGroup && <div className="field span-2"><label className="check-row"><input type="checkbox" checked={Boolean(form.inheritAuth)} onChange={(e) => update("inheritAuth", e.target.checked)} />沿用分组认证{inheritedProfile ? `：${inheritedProfile.name}` : "（分组当前未设置认证身份）"}</label></div>}
              <div className="field span-2">
                <label htmlFor="auth-method">认证方式</label>
                <select className="select w-full" id="auth-method" name="authMethod" disabled={form.inheritAuth} value={form.inheritAuth ? inheritedProfile?.authMethod ?? "agent" : form.authMethod} onChange={(e) => update("authMethod", e.target.value as AuthMethod)}>
                  <option value="key">SSH 私钥</option>
                  <option value="password">密码</option>
                  <option value="agent">SSH Agent</option>
                </select>
              </div>
              {!form.inheritAuth && form.authMethod === "key" && <>
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
              {!form.inheritAuth && form.authMethod === "password" && <div className="field span-2">
                <label htmlFor="server-password">SSH 密码{server ? "，留空表示保留" : "，必填"}</label>
                <input className="input" id="server-password" name="password" type="password" autoComplete="new-password" required={!server} value={form.password ?? ""} onChange={(e) => update("password", e.target.value)} />
                <p className="field-help">密码保存在 macOS Keychain，不会写入 SQLite。</p>
              </div>}
              {(form.inheritAuth ? inheritedProfile?.authMethod === "agent" : form.authMethod === "agent") && <p className="text-muted span-2">使用当前系统 SSH Agent 中已加载的身份，请确认 `ssh-add -L` 能列出密钥。</p>}
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
