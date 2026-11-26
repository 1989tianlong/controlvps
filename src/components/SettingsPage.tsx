import { useEffect, useState } from "react";
import { ArrowUpRight, Bell, LockKeyhole, Save } from "lucide-react";
import { api, errorMessage, openExternal } from "../lib/api";
import { sendSystemNotification } from "../lib/notifications";
import type { AppInfo, MonitoringSettings } from "../types";
import type { AppUpdate } from "../hooks/useAppUpdate";
import { BackupSettings } from "./BackupSettings";

interface Props {
  settings?: MonitoringSettings;
  onSettingsChange: (settings: MonitoringSettings) => void;
  appUpdate: AppUpdate;
  onWorkspaceRestored: () => void;
}

export function SettingsPage({ settings, onSettingsChange, appUpdate, onWorkspaceRestored }: Props) {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [draft, setDraft] = useState<MonitoringSettings>();
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");

  useEffect(() => { api.appInfo().then(setInfo).catch((reason) => setError(errorMessage(reason))); }, []);
  useEffect(() => { setDraft(settings); }, [settings]);

  const updateStatus = (() => {
    switch (appUpdate.state.status) {
      case "checking": return "正在检查";
      case "current": return "已是最新版本";
      case "available": return `可更新到 v${appUpdate.state.version}`;
      case "downloading": return appUpdate.state.percent === null ? "正在下载" : `正在下载 ${appUpdate.state.percent}%`;
      case "ready": return "已下载，重启后生效";
      case "error": return "检查失败";
      default: return "";
    }
  })();

  const update = <K extends keyof MonitoringSettings>(key: K, value: MonitoringSettings[K]) => {
    setDraft((current) => current ? { ...current, [key]: value } : current);
    setNotice("");
  };

  const save = async () => {
    if (!draft) return;
    setSaving(true);
    setError("");
    try {
      const saved = await api.updateMonitoringSettings(draft);
      setDraft(saved);
      onSettingsChange(saved);
      setNotice("监控与告警设置已保存");
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setSaving(false);
    }
  };

  const testNotification = async () => {
    setNotice("");
    setError("");
    try {
      const sent = await sendSystemNotification("ControlVPS 通知测试", "通知权限正常。阈值或离线告警会显示在这里。");
      setNotice(sent ? "测试通知已发送" : "macOS 通知权限未开启，请在系统设置中允许 ControlVPS 通知");
    } catch (reason) {
      setError(errorMessage(reason));
    }
  };

  return <>
    <div className="page-heading"><div><h1>设置</h1><p>配置本地监控、告警通知和数据存储。</p></div><button className="btn btn-primary" type="button" onClick={save} disabled={!draft || saving}><Save size={16} />{saving ? "正在保存" : "保存设置"}</button></div>
    {error && <div className="error-banner" role="alert">{error}</div>}
    {notice && <div className="panel panel-body mb-6" role="status">{notice}</div>}
    <div className="settings-list">
      {!draft ? <div className="skeleton settings-skeleton" /> : <>
        <div className="setting-row"><div><h2>自动监控全部 VPS</h2><p>应用运行期间按顺序分批采集，最多同时连接 2 台。首次未知主机密钥仍需在“我的 VPS”中手动确认。</p></div><label className="switch-control"><input type="checkbox" checked={draft.autoCollect} onChange={(event) => update("autoCollect", event.target.checked)} /><span>{draft.autoCollect ? "已开启" : "已关闭"}</span></label></div>
        <div className="setting-row"><div><h2>监控采集间隔</h2><p>每次采集都会保存历史采样。密集采集可能增加 SSH 连接和服务器负载。</p></div><select className="select justify-self-end" aria-label="监控采集间隔" value={draft.intervalSeconds} disabled={!draft.autoCollect} onChange={(event) => update("intervalSeconds", Number(event.target.value))}><option value={15}>每 15 秒</option><option value={30}>每 30 秒</option><option value={60}>每 60 秒</option><option value={300}>每 5 分钟</option><option value={900}>每 15 分钟</option><option value={3600}>每 1 小时</option></select></div>
        <div className="setting-row"><div><h2>阈值与离线告警</h2><p>达到阈值时触发，恢复后自动记录；同一活动告警在冷却期内不重复通知。</p></div><label className="switch-control"><input type="checkbox" checked={draft.alertsEnabled} onChange={(event) => update("alertsEnabled", event.target.checked)} /><span>{draft.alertsEnabled ? "已开启" : "已暂停"}</span></label></div>
        <div className="setting-row"><div><h2>资源告警阈值</h2><p>分别设置 CPU、内存和系统盘使用率百分比。</p></div><div className="threshold-grid">{([["cpuThreshold", "CPU"], ["memoryThreshold", "内存"], ["diskThreshold", "磁盘"]] as const).map(([key, label]) => <label key={key}><span>{label}</span><span className="threshold-input"><input className="input" type="number" min={1} max={100} step={1} value={draft[key]} disabled={!draft.alertsEnabled} onChange={(event) => update(key, Number(event.target.value))} /><span>%</span></span></label>)}</div></div>
        <div className="setting-row"><div><h2>离线判定</h2><p>连续采集失败达到次数后告警，避免因单次网络抖动打扰。</p></div><select className="select justify-self-end" aria-label="离线判定次数" value={draft.offlineAfter} disabled={!draft.alertsEnabled} onChange={(event) => update("offlineAfter", Number(event.target.value))}><option value={1}>连续 1 次失败</option><option value={2}>连续 2 次失败</option><option value={3}>连续 3 次失败</option><option value={5}>连续 5 次失败</option><option value={10}>连续 10 次失败</option></select></div>
        <div className="setting-row"><div><h2>重复告警冷却</h2><p>告警持续存在时，经过该时间才再次记录和通知。</p></div><select className="select justify-self-end" aria-label="告警冷却时间" value={draft.cooldownMinutes} disabled={!draft.alertsEnabled} onChange={(event) => update("cooldownMinutes", Number(event.target.value))}><option value={5}>5 分钟</option><option value={15}>15 分钟</option><option value={30}>30 分钟</option><option value={60}>1 小时</option><option value={360}>6 小时</option><option value={1440}>24 小时</option></select></div>
        <div className="setting-row"><div><h2>macOS 系统通知</h2><p>新触发的主机告警、服务故障和恢复事件可发送系统通知，事件始终保存在本地。应用关闭后不采集也不通知。</p></div><div className="setting-actions"><label className="switch-control"><input type="checkbox" checked={draft.notificationsEnabled} onChange={(event) => update("notificationsEnabled", event.target.checked)} /><span>{draft.notificationsEnabled ? "已开启" : "已关闭"}</span></label><button className="btn" type="button" onClick={testNotification}><Bell size={15} />测试通知</button></div></div>
      </>}
      <div className="setting-row"><div><h2>凭据存储</h2><p>密码与私钥密码由操作系统安全存储管理。</p></div><div className="setting-value flex items-center gap-2"><LockKeyhole size={16} />{info?.credentialStore ?? "正在读取"}</div></div>
      <div className="setting-row"><div><h2>本地数据目录</h2><p>包含服务器清单、主机与服务历史、告警事件和操作记录。</p></div><code className="setting-value measured">{info?.dataDir ?? "正在读取"}</code></div>
      <div className="setting-row"><div><h2>可信主机密钥</h2><p>首次确认后保存指纹对应的公钥，密钥变化会触发安全错误。</p></div><code className="setting-value measured">{info?.knownHostsPath ?? "正在读取"}</code></div>
      <div className="setting-row"><div><h2>开源许可证</h2><p>ControlVPS 使用 GNU General Public License v3.0。</p></div><button className="btn justify-self-end" type="button" onClick={() => openExternal("https://github.com/1989tianlong/controlvps")}>查看仓库<ArrowUpRight size={15} /></button></div>
      <div className="setting-row"><div><h2>应用更新</h2><p>启动后自动检查 GitHub 已发布版本；安装包通过签名校验后，重启应用生效。</p></div><div className="setting-actions">{updateStatus && <span className="measured">{updateStatus}</span>}<span className="measured">v{info?.version ?? "0.1.0"}</span>{appUpdate.state.status === "ready" ? <button className="btn btn-primary" type="button" onClick={() => void appUpdate.restart()}>重启应用</button> : <>{appUpdate.state.status === "available" && <button className="btn btn-primary" type="button" onClick={() => void appUpdate.install()}>下载并安装</button>}<button className="btn" type="button" disabled={appUpdate.state.status === "checking" || appUpdate.state.status === "downloading"} onClick={() => void appUpdate.checkForUpdate(true)}>{appUpdate.state.status === "checking" ? "正在检查" : "检查更新"}</button></>}</div></div>
    </div>
    <BackupSettings onRestored={onWorkspaceRestored} />
  </>;
}
