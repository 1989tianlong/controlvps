import { useCallback, useEffect, useState } from "react";
import { Cloud, KeyRound, RefreshCw, Trash2 } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import type { AuthMethod, AuthProfile, CloudAccount, CloudPreview, CloudProvider, Server, ServerGroup } from "../types";

interface Props {
  servers: Server[];
  onChanged: () => Promise<void>;
}

const providers: { value: CloudProvider; label: string }[] = [
  { value: "digital_ocean", label: "DigitalOcean" },
  { value: "tailscale", label: "Tailscale" },
];

export function AssetsPage({ servers, onChanged }: Props) {
  const [accounts, setAccounts] = useState<CloudAccount[]>([]);
  const [profiles, setProfiles] = useState<AuthProfile[]>([]);
  const [groups, setGroups] = useState<ServerGroup[]>([]);
  const [selectedAccount, setSelectedAccount] = useState("");
  const [preview, setPreview] = useState<CloudPreview>();
  const [selectedResources, setSelectedResources] = useState<string[]>([]);
  const [groupName, setGroupName] = useState("");
  const [busy, setBusy] = useState("");
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");

  const load = useCallback(async () => {
    const [nextAccounts, nextProfiles, nextGroups] = await Promise.all([
      api.cloudAccounts(),
      api.authProfiles(),
      api.serverGroups(),
    ]);
    setAccounts(nextAccounts);
    setProfiles(nextProfiles);
    setGroups(nextGroups);
    setSelectedAccount((current) => current || nextAccounts[0]?.id || "");
  }, []);

  useEffect(() => {
    void load().catch((reason) => setError(errorMessage(reason)));
  }, [load]);

  const run = async (key: string, operation: () => Promise<void>) => {
    setBusy(key);
    setError("");
    setNotice("");
    try { await operation(); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };

  const saveAccount = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const form = event.currentTarget;
    void run("account", async () => {
      await api.createCloudAccount({
        name: String(data.get("name") ?? ""),
        provider: String(data.get("provider") ?? "digital_ocean") as CloudProvider,
        token: String(data.get("token") ?? ""),
      });
      form.reset();
      await load();
      setNotice("云账户已保存，访问令牌只写入 macOS Keychain。");
    });
  };

  const saveProfile = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const form = event.currentTarget;
    void run("profile", async () => {
      await api.createAuthProfile({
        name: String(data.get("name") ?? ""),
        authMethod: String(data.get("authMethod") ?? "key") as AuthMethod,
        privateKeyPath: String(data.get("privateKeyPath") ?? ""),
        password: String(data.get("password") ?? ""),
        keyPassphrase: String(data.get("keyPassphrase") ?? ""),
      });
      form.reset();
      await load();
      setNotice("共享认证身份已保存。");
    });
  };

  const saveGroup = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const form = event.currentTarget;
    void run("group", async () => {
      const port = Number(data.get("defaultPort") ?? 0);
      await api.createServerGroup({
        name: String(data.get("name") ?? ""),
        jumpServerId: String(data.get("jumpServerId") ?? "") || undefined,
        authProfileId: String(data.get("authProfileId") ?? "") || undefined,
        defaultPort: port > 0 ? port : undefined,
        defaultUsername: String(data.get("defaultUsername") ?? "") || undefined,
      });
      form.reset();
      await load();
      setNotice("分组默认连接已保存。未单独配置的 VPS 会继承这些设置。");
    });
  };

  const sync = () => void run("sync", async () => {
    const result = await api.previewCloudAssets(selectedAccount);
    setPreview(result);
    setSelectedResources(result.changes.filter((item) => item.kind === "added" && item.resource.host).map((item) => item.resource.resourceId));
    await load();
  });

  const apply = () => void run("import", async () => {
    if (!preview) return;
    const result = await api.importCloudAssets({
      accountId: preview.accountId,
      resourceIds: selectedResources,
      groupName: groupName || undefined,
    });
    await onChanged();
    await load();
    setNotice(`已新增 ${result.created} 台，更新 ${result.updated} 台，跳过 ${result.skipped} 台。云端消失的机器不会被删除。`);
    setPreview(await api.previewCloudAssets(preview.accountId));
  });

  const toggle = (id: string) => {
    setSelectedResources((current) => current.includes(id) ? current.filter((item) => item !== id) : [...current, id]);
  };

  return <>
    <div className="page-heading"><div><h1>云资产</h1><p>只读同步 DigitalOcean 和 Tailscale，导入前先核对差异。令牌保存在 macOS Keychain。</p></div></div>
    {error && <div className="error-banner" role="alert">{error}</div>}
    {notice && <div className="panel panel-body mb-6" role="status">{notice}</div>}
    <div className="split-layout">
      <section className="panel panel-body">
        <h2>云账户</h2>
        <form className="stack mt-4" onSubmit={saveAccount}>
          <input className="input" name="name" required placeholder="账户名称" aria-label="云账户名称" />
          <select className="select" name="provider" aria-label="云服务商">{providers.map((item) => <option key={item.value} value={item.value}>{item.label}</option>)}</select>
          <input className="input" name="token" type="password" required autoComplete="off" placeholder="只读访问令牌" aria-label="云访问令牌" />
          <button className="btn btn-primary" type="submit" disabled={busy === "account"}>保存账户</button>
        </form>
        <div className="stack mt-6">{accounts.map((account) => <div className="list-row" key={account.id}><div><strong>{account.name}</strong><span className="table-meta">{providers.find((item) => item.value === account.provider)?.label} · {account.lastSyncedAt ? `上次同步 ${new Date(account.lastSyncedAt).toLocaleString()}` : "尚未同步"}</span></div><button className="icon-btn" type="button" aria-label={`删除 ${account.name}`} onClick={() => void run(account.id, async () => { await api.deleteCloudAccount(account.id); await load(); })}><Trash2 size={16} /></button></div>)}</div>
      </section>
      <section className="panel panel-body">
        <h2><KeyRound size={16} /> 分组与认证</h2>
        <form className="stack mt-4" onSubmit={saveProfile}>
          <input className="input" name="name" required placeholder="认证身份名称" aria-label="认证身份名称" />
          <select className="select" name="authMethod" aria-label="认证方式"><option value="key">SSH 私钥</option><option value="agent">SSH Agent</option><option value="password">密码</option></select>
          <input className="input" name="privateKeyPath" placeholder="私钥路径，例如 ~/.ssh/id_ed25519" aria-label="私钥路径" />
          <input className="input" name="password" type="password" autoComplete="off" placeholder="密码（可选）" aria-label="共享密码" />
          <button className="btn" type="submit" disabled={busy === "profile"}>保存认证身份</button>
        </form>
        <form className="stack mt-6" onSubmit={saveGroup}>
          <input className="input" name="name" required placeholder="分组名称" aria-label="分组名称" />
          <select className="select" name="jumpServerId" aria-label="默认跳板机"><option value="">不使用默认跳板机</option>{servers.filter((server) => !server.jumpServerId).map((server) => <option key={server.id} value={server.id}>{server.name}</option>)}</select>
          <select className="select" name="authProfileId" aria-label="默认认证身份"><option value="">不继承认证身份</option>{profiles.map((profile) => <option key={profile.id} value={profile.id}>{profile.name}</option>)}</select>
          <div className="form-grid"><input className="input" name="defaultUsername" placeholder="默认用户名" aria-label="默认用户名" /><input className="input" name="defaultPort" type="number" min={1} max={65535} placeholder="默认端口" aria-label="默认端口" /></div>
          <button className="btn" type="submit" disabled={busy === "group"}>保存分组默认值</button>
        </form>
        <div className="stack mt-6">{groups.map((group) => {
          const jump = servers.find((server) => server.id === group.jumpServerId);
          const profile = profiles.find((item) => item.id === group.authProfileId);
          return <div className="list-row" key={group.id}><div><strong>{group.name}</strong><span className="table-meta">{group.defaultUsername || "未设置用户名"}:{group.defaultPort || "未设置端口"}{jump ? ` · 跳板 ${jump.name}` : group.jumpServerId ? " · 跳板机已删除" : " · 直接连接"}{profile ? ` · 认证 ${profile.name}` : group.authProfileId ? " · 认证身份已删除" : ""}</span></div><button className="icon-btn" type="button" aria-label={`删除分组 ${group.name}`} onClick={() => void run(group.id, async () => { await api.deleteServerGroup(group.id); await load(); })}><Trash2 size={16} /></button></div>;
        })}</div>
      </section>
    </div>
    <section className="panel panel-body mt-6">
      <div className="section-heading"><h2><Cloud size={16} /> 同步预览</h2><div className="heading-actions"><select className="select" aria-label="选择云账户" value={selectedAccount} onChange={(event) => { setSelectedAccount(event.target.value); setPreview(undefined); }}>{accounts.map((account) => <option key={account.id} value={account.id}>{account.name}</option>)}</select><button className="btn" type="button" disabled={!selectedAccount || busy === "sync"} onClick={sync}><RefreshCw size={16} />{busy === "sync" ? "正在同步" : "获取差异"}</button></div></div>
      {!preview ? <p className="field-help mt-4">同步只读取云端清单，不会修改或删除本地 VPS。</p> : <>
        <p className="field-help mt-4">新增 {preview.added} · 变化 {preview.changed} · 云端消失 {preview.missing} · 未变化 {preview.unchanged}</p>
        <div className="filters mt-4"><select className="select" aria-label="导入到分组" value={groupName} onChange={(event) => setGroupName(event.target.value)}><option value="">导入时不指定分组</option>{groups.map((group) => <option key={group.id}>{group.name}</option>)}</select><button className="btn btn-primary" type="button" disabled={selectedResources.length === 0 || busy === "import"} onClick={apply}>{busy === "import" ? "正在应用" : `应用所选 ${selectedResources.length} 项`}</button></div>
        <div className="table-wrap mt-4"><table className="data-table"><thead><tr><th>选择</th><th>差异</th><th>资产</th><th>说明</th></tr></thead><tbody>{preview.changes.map((change) => <tr key={`${change.kind}-${change.resource.resourceId}`}><td>{change.kind === "missing" || !change.resource.host ? "—" : <input type="checkbox" checked={selectedResources.includes(change.resource.resourceId)} aria-label={`选择 ${change.resource.name}`} onChange={() => toggle(change.resource.resourceId)} />}</td><td>{{ added: "新增", changed: "变化", missing: "云端消失" }[change.kind]}</td><td><span className="table-name">{change.resource.name}</span><span className="table-meta measured">{change.resource.host || "无地址"} · {change.resource.region || "未知地区"} · {change.resource.status}</span></td><td>{change.summary}</td></tr>)}</tbody></table></div>
      </>}
    </section>
  </>;
}
