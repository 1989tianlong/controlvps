import { useEffect, useState } from "react";
import { ArrowUpRight, LockKeyhole } from "lucide-react";
import { api, errorMessage, openExternal } from "../lib/api";
import type { AppInfo } from "../types";

export function SettingsPage() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [error, setError] = useState("");
  const [interval, setIntervalValue] = useState(localStorage.getItem("controlvps.monitorInterval") ?? "30");
  useEffect(() => { api.appInfo().then(setInfo).catch((reason) => setError(errorMessage(reason))); }, []);
  const updateInterval = (value: string) => {
    setIntervalValue(value);
    localStorage.setItem("controlvps.monitorInterval", value);
  };
  return <>
    <div className="page-heading"><div><h1>设置</h1><p>管理本地采集行为和查看数据存储位置。</p></div></div>
    {error && <div className="error-banner" role="alert">{error}</div>}
    <div className="settings-list">
      <div className="setting-row"><div><h2>监控采集间隔</h2><p>仅对已经手动连接成功的当前服务器生效。应用关闭后不会继续采集。</p></div><select className="select justify-self-end" aria-label="监控采集间隔" value={interval} onChange={(e) => updateInterval(e.target.value)}><option value="0">关闭自动采集</option><option value="15">每 15 秒</option><option value="30">每 30 秒</option><option value="60">每 60 秒</option><option value="300">每 5 分钟</option></select></div>
      <div className="setting-row"><div><h2>凭据存储</h2><p>密码与私钥密码由操作系统安全存储管理。</p></div><div className="setting-value flex items-center gap-2"><LockKeyhole size={16} />{info?.credentialStore ?? "正在读取"}</div></div>
      <div className="setting-row"><div><h2>本地数据目录</h2><p>包含服务器清单、操作记录与 SQLite 数据库。</p></div><code className="setting-value measured">{info?.dataDir ?? "正在读取"}</code></div>
      <div className="setting-row"><div><h2>可信主机密钥</h2><p>首次确认后保存指纹对应的公钥，密钥变化会触发安全错误。</p></div><code className="setting-value measured">{info?.knownHostsPath ?? "正在读取"}</code></div>
      <div className="setting-row"><div><h2>开源许可证</h2><p>ControlVPS 使用 GNU General Public License v3.0。</p></div><button className="btn justify-self-end" type="button" onClick={() => openExternal("https://github.com/1989tianlong/controlvps")}>查看仓库<ArrowUpRight size={15} /></button></div>
      <div className="setting-row"><div><h2>应用版本</h2><p>Rust、Tauri 2、React 与 Tailwind CSS。</p></div><span className="measured justify-self-end">v{info?.version ?? "0.1.0"}</span></div>
    </div>
  </>;
}
