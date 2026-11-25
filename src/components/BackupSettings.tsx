import { useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { ArchiveRestore, Download, Eye, FileKey2, ShieldCheck, Upload } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import type { BackupPreview } from "../types";

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
      <article className="panel backup-card"><div className="backup-card-heading"><Download size={19} /><div><h3>导出工作区</h3><p>包含 VPS 配置、服务监控、Runbook、命令片段、隧道和监控设置；不包含私钥文件、历史指标和审计记录。</p></div></div><div className="form-grid"><div className="field"><label htmlFor="backup-password">备份密码</label><input id="backup-password" className="input" type="password" value={exportPassword} onChange={(event) => setExportPassword(event.target.value)} autoComplete="new-password" placeholder="至少 8 个字符" /></div><div className="field"><label htmlFor="backup-confirmation">确认密码</label><input id="backup-confirmation" className="input" type="password" value={exportConfirmation} onChange={(event) => setExportConfirmation(event.target.value)} autoComplete="new-password" aria-invalid={exportConfirmation.length > 0 && exportPassword !== exportConfirmation} /></div></div><label className="backup-credential-option"><input type="checkbox" checked={includeCredentials} onChange={(event) => setIncludeCredentials(event.target.checked)} /><span><strong>同时包含 Keychain 中的密码、私钥密码和 Webhook URL</strong><small>默认不包含。选中后凭据只会进入加密备份，不会明文写入磁盘。</small></span></label><button className="btn btn-primary" type="button" disabled={busy !== "" || exportPassword.length < 8 || exportPassword !== exportConfirmation} onClick={() => void exportBackup()}><FileKey2 size={16} />{busy === "export" ? "正在加密导出" : "选择位置并导出"}</button></article>
      <article className="panel backup-card"><div className="backup-card-heading"><Upload size={19} /><div><h3>合并恢复</h3><p>先解密预览，再确认恢复。相同主机、端口和用户的 VPS 会跳过，不覆盖现有资产。</p></div></div><div className="field"><label htmlFor="restore-file">备份文件</label><div className="backup-file-picker"><input id="restore-file" className="input" value={restorePath} readOnly placeholder="尚未选择文件" /><button className="btn" type="button" disabled={busy !== ""} onClick={() => void selectBackup()}>选择文件</button></div></div><div className="field mt-4"><label htmlFor="restore-password">备份密码</label><input id="restore-password" className="input" type="password" value={restorePassword} onChange={(event) => { setRestorePassword(event.target.value); setPreview(undefined); }} autoComplete="off" /></div><div className="heading-actions mt-4"><button className="btn" type="button" disabled={busy !== "" || !restorePath || restorePassword.length < 8} onClick={() => void inspect()}><Eye size={16} />{busy === "preview" ? "正在解密" : "验证并预览"}</button><button className="btn btn-primary" type="button" disabled={busy !== "" || !preview} onClick={() => void restore()}><ArchiveRestore size={16} />{busy === "restore" ? "正在恢复" : "确认合并恢复"}</button></div>{preview && <div className="backup-preview" role="status"><strong>备份验证成功</strong><span>{preview.serverCount} 台 VPS：新增 {preview.newServerCount}，已有 {preview.existingServerCount}</span><span>{preview.snippetCount} 个命令片段 · {preview.runbookCount} 个 Runbook · {preview.portForwardCount} 条隧道 · {preview.serviceMonitorCount} 个服务监控</span><span>{preview.includesCredentials ? "包含加密凭据" : "不包含凭据"} · 恢复会更新监控设置</span></div>}</article>
    </div>
  </section>;
}
