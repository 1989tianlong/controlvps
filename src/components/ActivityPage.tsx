import { useCallback, useEffect, useState } from "react";
import { RefreshCw, ScrollText } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import { formatTime } from "../lib/format";
import type { OperationLog } from "../types";

const actionNames: Record<string, string> = {
  SystemInfo: "系统信息",
  Processes: "高占用进程",
  DiskUsage: "磁盘与 inode",
  SystemLogs: "最近系统日志",
  CleanCache: "清理包缓存",
  Reboot: "重启服务器",
};

export function ActivityPage() {
  const [logs, setLogs] = useState<OperationLog[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const load = useCallback(async () => {
    setLoading(true);
    setError("");
    try { setLogs(await api.operationLogs()); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setLoading(false); }
  }, []);
  useEffect(() => { void load(); }, [load]);
  return <>
    <div className="page-heading"><div><h1>操作记录</h1><p>记录快捷运维的动作、结果和时间，不记录终端内容或凭据。</p></div><button className="btn" type="button" onClick={load} disabled={loading}><RefreshCw size={16} />刷新</button></div>
    {error && <div className="error-banner" role="alert">{error}</div>}
    {loading ? <div className="skeleton" /> : logs.length === 0 ? <div className="empty"><div><ScrollText size={30} /><h2>还没有快捷运维记录</h2><p>在服务器详情的“快捷运维”中执行操作后，结果会显示在这里。</p></div></div> : <div className="table-wrap panel"><table className="data-table"><thead><tr><th>时间</th><th>服务器</th><th>操作</th><th>结果</th><th>摘要</th></tr></thead><tbody>{logs.map((log) => <tr key={log.id}><td className="log-time">{formatTime(log.createdAt)}</td><td className="table-name">{log.serverName}</td><td>{actionNames[log.action] ?? log.action}</td><td><span className={log.success ? "badge badge-ok" : "badge badge-bad"}>{log.success ? "成功" : "失败"}</span></td><td>{log.summary}</td></tr>)}</tbody></table></div>}
  </>;
}
