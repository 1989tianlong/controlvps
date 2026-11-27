import { useEffect, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { ArchiveRestore, Cloud, Download, Eye, FileKey2, FolderSync, KeyRound, RefreshCw, ShieldCheck, Upload } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import { formatBytes, formatTime } from "../lib/format";
import type { BackupPreview, ByoSyncStatus, SecretProviderStatus } from "../types";

export function BackupSettings({ onRestored }: { onRestored: () => void }) {
  const [exportPassword, setExportPassword] = useState("");
  const [exportConfirmation, setExportConfirmation] = useState("");
  const [includeCredentials, setIncludeCredentials] = useState(false);
  const [restorePath, setRestorePath] = useState("");
  const [restorePassword, setRestorePassword] = useState("");
  const [preview, setPreview] = useState<BackupPreview>();
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");

  // BYO-Sync state
  const [syncStatus, setSyncStatus] = useState<ByoSyncStatus>();
  const [syncPassword, setSyncPassword] = useState("");
  const [syncIncludeCreds, setSyncIncludeCreds] = useState(false);

  // External secret providers (1Password / Bitwarden)
  const [providers, setProviders] = useState<SecretProviderStatus[]>([]);
  const [detectingProviders, setDetectingProviders] = useState(false);
  const [testRefInput, setTestRefInput] = useState("");
  const [testRefOutput, setTestRefOutput] = useState("");

  const refreshSync = async () => {
    try {
      const status = await api.getByoSyncStatus();
      setSyncStatus(status);
      setSyncIncludeCreds(status.config.includeCredentials);
    } catch {
      // Ignore outside Tauri
    }
  };

  const refreshProviders = async () => {
    setDetectingProviders(true);
    try {
      setProviders(await api.detectSecretProviders());
    } catch {
      // Ignore outside Tauri
    } finally {
      setDetectingProviders(false);
    }
  };

  useEffect(() => {
    void refreshSync();
    void refreshProviders();
  }, []);

  const chooseSyncFolder = async () => {
    setError("");
    setNotice("");
    try {
      const selected = await open({ title: "选择加密同步目录（iCloud Drive / Dropbox / Syncthing / Git）", multiple: false, directory: true });
      if (typeof selected !== "string") return;
      const updated = await api.updateByoSyncConfig(selected, syncIncludeCreds);
      setSyncStatus(updated);
      setNotice(`已配置 BYO-Sync 同步目录：${selected}`);
    } catch (reason) {
      setError(errorMessage(reason));
    }
  };

  const toggleSyncCredentials = async (checked: boolean) => {
    setSyncIncludeCreds(checked);
    if (!syncStatus?.config.folderPath) return;
    try {
      setSyncStatus(await api.updateByoSyncConfig(syncStatus.config.folderPath, checked));
    } catch (reason) {
      setError(errorMessage(reason));
    }
  };

  const pushSync = async () => {
    if (syncPassword.length < 8) return;
    setBusy("sync-push");
    setError("");
    setNotice("");
    try {
      const updated = await api.byoSyncPush(syncPassword);
      setSyncStatus(updated);
      setNotice(`已加密推送到同步目录：${updated.vaultFilePath ?? ""}`);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setSyncPassword("");
      setBusy("");
    }
  };

  const pullSync = async () => {
    if (syncPassword.length < 8) return;
    if (!window.confirm("确认从同步目录解密并合并恢复？现有 VPS 与 Runbook 不会被覆盖。")) return;
    setBusy("sync-pull");
    setError("");
    setNotice("");
    try {
      const result = await api.byoSyncPull(syncPassword);
      onRestored();
      await refreshSync();
      setNotice(`同步拉取完成：新增 ${result.importedServers} 台 VPS、${result.importedRunbooks} 个 Runbook、${result.importedSnippets} 个命令片段；跳过 ${result.skippedServers} 台已有 VPS。`);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setSyncPassword("");
      setBusy("");
    }
  };

  const runSecretRefTest = async () => {
    if (!testRefInput.trim()) return;
    setBusy("test-secret");
    setTestRefOutput("");
    setError("");
    try {
      const res = await api.testSecretRef(testRefInput.trim());
      setTestRefOutput(`${res.provider} 解析成功：${res.preview}（耗时 ${res.elapsedMs} ms）`);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy("");
    }
  };

  const exportBackup = async () => {
    if (exportPassword.length < 8 || exportPassword !== exportConfirmation) return;
    setBusy("export");
    setError("");
    setNotice("");
    try {
      const path = await save({
        title: "保存加密备份",
        defaultPath: `ControlVPS-${new Date().toISOString().slice(0, 10)}.cvpsbackup`,
        filters: [{ name: "ControlVPS 加密备份", extensions: ["cvpsbackup"] }],
      });
      if (!path) return;
      const summary = await api.createEncryptedBackup(path, exportPassword, includeCredentials);
      setNotice(`加密备份已保存：${summary.path} · ${summary.serverCount} 台 VPS，${summary.runbookCount} 个 Runbook，${summary.serviceMonitorCount} 个服务监控，${(summary.sizeBytes / 1024).toFixed(1)} KiB`);
    } catch (reason) { setError(errorMessage(reason)); }
    finally {
      setExportPassword("");
      setExportConfirmation("");
      setBusy("");
    }
  };
  const selectBackup = async () => {
    setError("");
    setNotice("");
    try {
      const path = await open({
        title: "选择 ControlVPS 加密备份",
        multiple: false,
        directory: false,
        filters: [{ name: "ControlVPS 加密备份", extensions: ["cvpsbackup"] }],
      });
      if (typeof path === "string") {
        setRestorePath(path);
        setPreview(undefined);
        setRestorePassword("");
      }
    } catch (reason) { setError(errorMessage(reason)); }
  };
  const inspect = async () => {
    if (!restorePath || restorePassword.length < 8) return;
    setBusy("preview");
    setError("");
    setNotice("");
    try { setPreview(await api.previewEncryptedBackup(restorePath, restorePassword)); }
    catch (reason) { setPreview(undefined); setError(errorMessage(reason)); }
    finally { setBusy(""); }
  };
  const restore = async () => {
    if (!preview || !window.confirm("确认合并恢复这个备份？现有 VPS、命令片段和 Runbook 不会被覆盖；监控设置会恢复为备份中的值。")) return;
    setBusy("restore");
    setError("");
    setNotice("");
    try {
      const result = await api.restoreEncryptedBackup(restorePath, restorePassword);
      onRestored();
      setNotice(`恢复完成：新增 ${result.importedServers} 台 VPS、${result.importedSnippets} 个命令片段、${result.importedRunbooks} 个 Runbook、${result.importedPortForwards} 条隧道、${result.importedServiceMonitors} 个服务监控；跳过 ${result.skippedServers} 台已有 VPS。`);
      setPreview(undefined);
      setRestorePath("");
    } catch (reason) { setError(errorMessage(reason)); }
    finally {
      setRestorePassword("");
      setBusy("");
    }
  };

  return <section className="backup-settings" aria-label="加密备份与恢复">
    <div className="section-heading"><div><h2>加密备份与恢复</h2><p className="table-meta">Argon2id + AES-256-GCM · 备份文件权限仅当前用户可读写</p></div><ShieldCheck size={21} /></div>
    {error && <div className="error-banner" role="alert">{error}</div>}
    {notice && <div className="file-notice backup-notice" role="status">{notice}</div>}
    <div className="backup-grid">
      <article className="panel backup-card"><div className="backup-card-heading"><Download size={19} /><div><h3>导出工作区</h3><p>包含 VPS 配置、服务监控、Runbook、命令片段、隧道和监控设置；不包含私钥文件、历史指标和审计记录。</p></div></div><div className="form-grid"><div className="field"><label htmlFor="backup-password">备份密码</label><input id="backup-password" className="input" type="password" value={exportPassword} onChange={(event) => setExportPassword(event.target.value)} autoComplete="new-password" placeholder="至少 8 个字符" /></div><div className="field"><label htmlFor="backup-confirmation">确认密码</label><input id="backup-confirmation" className="input" type="password" value={exportConfirmation} onChange={(event) => setExportConfirmation(event.target.value)} autoComplete="new-password" aria-invalid={exportConfirmation.length > 0 && exportPassword !== exportConfirmation} /></div></div><label className="backup-credential-option"><input type="checkbox" checked={includeCredentials} onChange={(event) => setIncludeCredentials(event.target.checked)} /><span><strong>同时包含 Keychain 中的密码、私钥密码、Compose 环境变量和 Webhook URL</strong><small>默认不包含。选中后凭据只会进入加密备份，不会明文写入磁盘。</small></span></label><button className="btn btn-primary" type="button" disabled={busy !== "" || exportPassword.length < 8 || exportPassword !== exportConfirmation} onClick={() => void exportBackup()}><FileKey2 size={16} />{busy === "export" ? "正在加密导出" : "选择位置并导出"}</button></article>
      <article className="panel backup-card"><div className="backup-card-heading"><Upload size={19} /><div><h3>合并恢复</h3><p>先解密预览，再确认恢复。相同主机、端口和用户的 VPS 会跳过，不覆盖现有资产。</p></div></div><div className="field"><label htmlFor="restore-file">备份文件</label><div className="backup-file-picker"><input id="restore-file" className="input" value={restorePath} readOnly placeholder="尚未选择文件" /><button className="btn" type="button" disabled={busy !== ""} onClick={() => void selectBackup()}>选择文件</button></div></div><div className="field mt-4"><label htmlFor="restore-password">备份密码</label><input id="restore-password" className="input" type="password" value={restorePassword} onChange={(event) => { setRestorePassword(event.target.value); setPreview(undefined); }} autoComplete="off" /></div><div className="heading-actions mt-4"><button className="btn" type="button" disabled={busy !== "" || !restorePath || restorePassword.length < 8} onClick={() => void inspect()}><Eye size={16} />{busy === "preview" ? "正在解密" : "验证并预览"}</button><button className="btn btn-primary" type="button" disabled={busy !== "" || !preview} onClick={() => void restore()}><ArchiveRestore size={16} />{busy === "restore" ? "正在恢复" : "确认合并恢复"}</button></div>{preview && <div className="backup-preview" role="status"><strong>备份验证成功</strong><span>{preview.serverCount} 台 VPS：新增 {preview.newServerCount}，已有 {preview.existingServerCount}</span><span>{preview.snippetCount} 个命令片段 · {preview.runbookCount} 个 Runbook · {preview.portForwardCount} 条隧道 · {preview.serviceMonitorCount} 个服务监控</span><span>{preview.includesCredentials ? "包含加密凭据" : "不包含凭据"} · 恢复会更新监控设置</span></div>}</article>
      <article className="panel backup-card">
        <div className="backup-card-heading">
          <FolderSync size={19} />
          <div>
            <h3>BYO-Sync 加密目录同步</h3>
            <p>指定 iCloud Drive、Dropbox、Syncthing 或私有 Git 目录，多台设备间零账号加密同步工作区。</p>
          </div>
        </div>
        <div className="field">
          <label htmlFor="byo-sync-dir">同步目录</label>
          <div className="backup-file-picker">
            <input id="byo-sync-dir" className="input" value={syncStatus?.config.folderPath ?? ""} readOnly placeholder="尚未选择同步目录" />
            <button className="btn" type="button" disabled={busy !== ""} onClick={() => void chooseSyncFolder()}>选择目录</button>
            <button className="btn" type="button" disabled={busy !== ""} onClick={() => void refreshSync()} title="检查目录变更"><RefreshCw size={15} /></button>
          </div>
        </div>
        {syncStatus?.config.folderPath && (
          <div className="mt-3 text-sm flex flex-wrap items-center gap-2">
            {syncStatus.remoteExists ? (
              <span className={syncStatus.remoteHasChanges ? "badge badge-warn" : "badge badge-ok"}>
                {syncStatus.remoteHasChanges ? "检测到远端新版本可拉取" : "与同步目录一致"}
              </span>
            ) : (
              <span className="badge">目录中尚无同步文件</span>
            )}
            {syncStatus.remoteSizeBytes != null && <span className="table-meta">{formatBytes(syncStatus.remoteSizeBytes)}</span>}
            {syncStatus.remoteUpdatedAt && <span className="table-meta">文件更新：{formatTime(syncStatus.remoteUpdatedAt)}</span>}
            {syncStatus.config.lastSyncedAt && <span className="table-meta">上次同步：{formatTime(syncStatus.config.lastSyncedAt)} ({syncStatus.config.lastDirection === "pull" ? "拉取" : "推送"})</span>}
          </div>
        )}
        <label className="backup-credential-option mt-3">
          <input type="checkbox" checked={syncIncludeCreds} onChange={(e) => void toggleSyncCredentials(e.target.checked)} />
          <span><strong>同步时包含加密凭据</strong><small>使用下方同步口令以 Argon2id + AES-256-GCM 端到端加密。</small></span>
        </label>
        <div className="field mt-3">
          <label htmlFor="byo-sync-pass">同步加密口令（至少 8 位）</label>
          <input id="byo-sync-pass" className="input" type="password" value={syncPassword} onChange={(e) => setSyncPassword(e.target.value)} autoComplete="off" placeholder="所有设备使用同一口令" />
        </div>
        <div className="heading-actions mt-4">
          <button className="btn btn-primary" type="button" disabled={busy !== "" || !syncStatus?.config.folderPath || syncPassword.length < 8} onClick={() => void pushSync()}>
            <Cloud size={15} />
            {busy === "sync-push" ? "正在加密推送" : "加密推送到目录"}
          </button>
          <button className="btn" type="button" disabled={busy !== "" || !syncStatus?.remoteExists || syncPassword.length < 8} onClick={() => void pullSync()}>
            <ArchiveRestore size={15} />
            {busy === "sync-pull" ? "正在拉取合并" : "从目录拉取合并"}
          </button>
        </div>
      </article>
      <article className="panel backup-card">
        <div className="backup-card-heading">
          <KeyRound size={19} />
          <div>
            <h3>外部密码管理器动态取密</h3>
            <p>在 VPS 密码或私钥口令中填写 <code>op://Vault/Item/Field</code> 或 <code>bw://条目名/password</code>，连接 SSH 时实时取密、不落盘明文。</p>
          </div>
        </div>
        <div className="flex flex-col gap-2 mb-3">
          {providers.map((p) => (
            <div key={p.provider} className="flex flex-wrap items-center justify-between gap-2 text-sm">
              <span><strong>{p.label}</strong> {p.version ? <span className="table-meta">v{p.version}</span> : null}</span>
              <span className={p.signedIn ? "badge badge-ok" : p.available ? "badge badge-warn" : "badge"}>
                {p.detail}
              </span>
            </div>
          ))}
        </div>
        <div className="field">
          <label htmlFor="secret-ref-test">测试凭据引用解析</label>
          <div className="backup-file-picker">
            <input id="secret-ref-test" className="input" value={testRefInput} onChange={(e) => setTestRefInput(e.target.value)} placeholder="例如 op://Personal/MyVPS/password 或 bw://MyVPS/password" />
            <button className="btn" type="button" disabled={busy !== "" || !testRefInput.trim()} onClick={() => void runSecretRefTest()}>
              {busy === "test-secret" ? "解析中" : "测试解析"}
            </button>
            <button className="btn" type="button" disabled={detectingProviders} onClick={() => void refreshProviders()} title="重新检测 CLI">
              <RefreshCw size={15} />
            </button>
          </div>
        </div>
        {testRefOutput && <div className="file-notice mt-3" role="status">{testRefOutput}</div>}
      </article>
    </div>
  </section>;
}
