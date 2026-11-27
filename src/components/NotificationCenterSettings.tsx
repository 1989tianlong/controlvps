import { useCallback, useEffect, useRef, useState } from "react";
import { BellRing, CheckCircle2, History, Pencil, Plus, Send, Trash2, X, XCircle } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import { formatTime } from "../lib/format";
import type {
  NotificationChannel,
  NotificationChannelInput,
  NotificationChannelKind,
  NotificationDelivery,
  NotificationEventKind,
  NotificationSeverity,
} from "../types";

const channelKindLabels: Record<NotificationChannelKind, string> = {
  telegram: "Telegram Bot",
  feishu: "飞书机器人",
  dingtalk: "钉钉机器人",
  wecom: "企业微信",
  ntfy: "ntfy",
  bark: "Bark (iOS)",
  webhook: "通用 Webhook",
};

const eventLabels: Record<NotificationEventKind, string> = {
  host_alert: "主机告警",
  service_alert: "服务故障",
  certificate_expiry: "证书预警",
  backup_failure: "备份失败",
  runbook_failure: "Runbook 失败",
  server_expiry: "VPS 到期",
};

const allEventKinds: NotificationEventKind[] = [
  "host_alert",
  "service_alert",
  "certificate_expiry",
  "backup_failure",
  "runbook_failure",
  "server_expiry",
];

const severityLabels: Record<NotificationSeverity, string> = {
  info: "全部（含恢复/信息）",
  warning: "警告及以上",
  critical: "仅严重故障",
};

const emptyInput: NotificationChannelInput = {
  name: "",
  kind: "telegram",
  enabled: true,
  target: "",
  secret: "",
  signingSecret: "",
  clearSigningSecret: false,
  events: [],
  minSeverity: "warning",
  quietStart: "",
  quietEnd: "",
};

function secretFieldMeta(kind: NotificationChannelKind) {
  switch (kind) {
    case "telegram":
      return { targetLabel: "Chat ID，必填", targetPlaceholder: "例如 -1001234567890", secretLabel: "Bot Token", secretPlaceholder: "123456:ABC-DEF...", hasSigning: false, secretRequired: true };
    case "feishu":
      return { targetLabel: "备注标识（可选）", targetPlaceholder: "例如 运维告警群", secretLabel: "飞书 Webhook URL", secretPlaceholder: "https://open.feishu.cn/open-apis/bot/v2/hook/...", hasSigning: true, secretRequired: true };
    case "dingtalk":
      return { targetLabel: "备注标识（可选）", targetPlaceholder: "例如 生产值班群", secretLabel: "钉钉 Webhook URL", secretPlaceholder: "https://oapi.dingtalk.com/robot/send?access_token=...", hasSigning: true, secretRequired: true };
    case "wecom":
      return { targetLabel: "备注标识（可选）", targetPlaceholder: "例如 运维通知群", secretLabel: "企业微信 Webhook URL", secretPlaceholder: "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=...", hasSigning: false, secretRequired: true };
    case "ntfy":
      return { targetLabel: "ntfy 主题 URL，必填", targetPlaceholder: "https://ntfy.sh/my-controlvps-alerts", secretLabel: "访问令牌（可选）", secretPlaceholder: "tk_...（私有主题填写）", hasSigning: false, secretRequired: false };
    case "bark":
      return { targetLabel: "Bark 服务地址", targetPlaceholder: "https://api.day.app", secretLabel: "Device Key", secretPlaceholder: "Bark App 中的设备密钥", hasSigning: false, secretRequired: true };
    case "webhook":
      return { targetLabel: "备注标识（可选）", targetPlaceholder: "例如 自建告警接收端", secretLabel: "Webhook URL", secretPlaceholder: "https://example.com/hooks/controlvps", hasSigning: true, secretRequired: true };
  }
}

export function NotificationCenterSettings() {
  const [channels, setChannels] = useState<NotificationChannel[]>([]);
  const [deliveries, setDeliveries] = useState<NotificationDelivery[]>([]);
  const [showDeliveries, setShowDeliveries] = useState(false);
  const [editorOpen, setEditorOpen] = useState(false);
  const [editing, setEditing] = useState<NotificationChannel | null>(null);
  const [draft, setDraft] = useState<NotificationChannelInput>(emptyInput);
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const dialogRef = useRef<HTMLDialogElement>(null);

  const load = useCallback(async () => {
    try {
      const [channelList, deliveryList] = await Promise.all([
        api.listNotificationChannels(),
        api.listNotificationDeliveries(30),
      ]);
      setChannels(channelList);
      setDeliveries(deliveryList);
    } catch (reason) {
      setError(errorMessage(reason));
    }
  }, []);

  useEffect(() => { void load(); }, [load]);
  useEffect(() => {
    if (editorOpen && !dialogRef.current?.open) dialogRef.current?.showModal();
    if (!editorOpen && dialogRef.current?.open) dialogRef.current.close();
  }, [editorOpen]);

  const openEditor = (channel?: NotificationChannel) => {
    setEditing(channel ?? null);
    setDraft(channel ? {
      name: channel.name,
      kind: channel.kind,
      enabled: channel.enabled,
      target: channel.target,
      secret: "",
      signingSecret: "",
      clearSigningSecret: false,
      events: channel.events,
      minSeverity: channel.minSeverity,
      quietStart: channel.quietStart ?? "",
      quietEnd: channel.quietEnd ?? "",
    } : emptyInput);
    setError("");
    setEditorOpen(true);
  };

  const toggleEvent = (kind: NotificationEventKind) => {
    setDraft((current) => ({
      ...current,
      events: current.events.includes(kind)
        ? current.events.filter((item) => item !== kind)
        : [...current.events, kind],
    }));
  };

  const save = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy("save");
    setError("");
    setNotice("");
    try {
      if (editing) await api.updateNotificationChannel(editing.id, draft);
      else await api.createNotificationChannel(draft);
      setEditorOpen(false);
      setNotice("通知渠道已保存，凭据仅写入系统钥匙串");
      await load();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy("");
    }
  };

  const toggleEnabled = async (channel: NotificationChannel) => {
    setBusy(`toggle-${channel.id}`);
    setError("");
    try {
      await api.setNotificationChannelEnabled(channel.id, !channel.enabled);
      await load();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy("");
    }
  };

  const testChannel = async (channel: NotificationChannel) => {
    setBusy(`test-${channel.id}`);
    setError("");
    setNotice("");
    try {
      await api.testNotificationChannel(channel.id);
      setNotice(`测试通知已通过「${channel.name}」发送成功`);
      await load();
    } catch (reason) {
      setError(`渠道「${channel.name}」发送失败：${errorMessage(reason)}`);
      await load();
    } finally {
      setBusy("");
    }
  };

  const remove = async (channel: NotificationChannel) => {
    if (!window.confirm(`确认删除通知渠道「${channel.name}」？钥匙串中的对应凭据也会一并清理。`)) return;
    setBusy(`delete-${channel.id}`);
    setError("");
    try {
      await api.deleteNotificationChannel(channel.id);
      setNotice(`已删除通知渠道「${channel.name}」`);
      await load();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy("");
    }
  };

  const clearDeliveries = async () => {
    try {
      await api.clearNotificationDeliveries();
      setDeliveries([]);
    } catch (reason) {
      setError(errorMessage(reason));
    }
  };

  const meta = secretFieldMeta(draft.kind);

  return <section className="backup-settings" aria-label="通知中心">
    <div className="section-heading">
      <div>
        <h2>通知中心 2.0</h2>
        <p className="table-meta">统一推送主机告警、服务故障、证书到期、Restic 备份失败、Runbook 异常与 VPS 到期提醒到手机和群聊</p>
      </div>
      <div className="heading-actions">
        <button className="btn" type="button" onClick={() => setShowDeliveries((value) => !value)}>
          <History size={15} />投递记录 ({deliveries.length})
        </button>
        <button className="btn btn-primary" type="button" onClick={() => openEditor()}>
          <Plus size={15} />添加渠道
        </button>
      </div>
    </div>
    {error && <div className="error-banner" role="alert">{error}</div>}
    {notice && <div className="file-notice backup-notice" role="status">{notice}</div>}
    {channels.length === 0 ? (
      <div className="panel panel-body">
        <div className="flex items-center justify-between gap-4">
          <div className="flex items-center gap-3">
            <BellRing size={20} />
            <div>
              <strong>尚未配置外部通知渠道</strong>
              <p className="table-meta">支持 Telegram、飞书、钉钉、企业微信、ntfy、Bark 与 HMAC Webhook；凭据保存在系统钥匙串，不写入 SQLite。</p>
            </div>
          </div>
          <button className="btn" type="button" onClick={() => openEditor()}>立即配置</button>
        </div>
      </div>
    ) : (
      <div className="table-wrap panel">
        <table className="data-table">
          <thead>
            <tr>
              <th>渠道</th>
              <th>路由与级别</th>
              <th>静默时段</th>
              <th>状态</th>
              <th className="text-right">操作</th>
            </tr>
          </thead>
          <tbody>
            {channels.map((channel) => (
              <tr key={channel.id}>
                <td>
                  <span className="table-name">{channel.name}</span>
                  <span className="table-meta">{channelKindLabels[channel.kind]}{channel.target ? ` · ${channel.target}` : ""}{channel.hasSigningSecret ? " · 已加签" : ""}</span>
                </td>
                <td>
                  <span className="table-meta">
                    {channel.events.length === 0 ? "全部事件" : channel.events.map((kind) => eventLabels[kind]).join("、")}
                  </span>
                  <span className="table-meta">{severityLabels[channel.minSeverity]}</span>
                </td>
                <td className="measured">{channel.quietStart && channel.quietEnd ? `${channel.quietStart}–${channel.quietEnd}` : "无"}</td>
                <td>
                  <button
                    className={channel.enabled ? "badge badge-ok" : "badge"}
                    type="button"
                    disabled={busy !== ""}
                    onClick={() => void toggleEnabled(channel)}
                  >
                    {channel.enabled ? "已启用" : "已停用"}
                  </button>
                </td>
                <td>
                  <div className="row-actions justify-end">
                    <button className="btn" type="button" disabled={busy !== ""} onClick={() => void testChannel(channel)}>
                      <Send size={14} />{busy === `test-${channel.id}` ? "发送中" : "测试"}
                    </button>
                    <button className="icon-btn" type="button" title="编辑渠道" aria-label={`编辑 ${channel.name}`} disabled={busy !== ""} onClick={() => openEditor(channel)}>
                      <Pencil size={15} />
                    </button>
                    <button className="icon-btn" type="button" title="删除渠道" aria-label={`删除 ${channel.name}`} disabled={busy !== ""} onClick={() => void remove(channel)}>
                      <Trash2 size={15} />
                    </button>
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    )}

    {showDeliveries && (
      <div className="panel mt-4">
        <div className="panel-header">
          <div><h3>最近投递记录</h3><span className="table-meta">失败自动指数退避重试最多 3 次</span></div>
          {deliveries.length > 0 && <button className="btn btn-danger" type="button" onClick={() => void clearDeliveries()}><Trash2 size={14} />清空</button>}
        </div>
        {deliveries.length === 0 ? (
          <p className="panel-body table-meta">暂无投递记录。</p>
        ) : (
          <div className="table-wrap">
            <table className="data-table">
              <thead><tr><th>时间</th><th>渠道</th><th>事件</th><th>结果</th><th>内容</th></tr></thead>
              <tbody>
                {deliveries.map((item) => (
                  <tr key={item.id}>
                    <td className="log-time">{formatTime(item.createdAt)}</td>
                    <td>{item.channelName}</td>
                    <td><span className="badge">{eventLabels[item.eventKind]}</span></td>
                    <td>
                      <span className={item.success ? "badge badge-ok" : "badge badge-bad"}>
                        {item.success ? <CheckCircle2 size={12} /> : <XCircle size={12} />}
                        {item.success ? `成功(${item.attempts})` : `失败(${item.attempts})`}
                      </span>
                    </td>
                    <td>
                      <span className="table-name">{item.title}</span>
                      {item.error && <span className="table-meta text-danger">{item.error}</span>}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    )}

    <dialog ref={dialogRef} className="dialog" onCancel={(event) => { event.preventDefault(); if (!busy) setEditorOpen(false); }}>
      <form onSubmit={(event) => void save(event)}>
        <div className="dialog-header">
          <h2>{editing ? "编辑通知渠道" : "添加通知渠道"}</h2>
          <button className="icon-btn" type="button" aria-label="关闭" onClick={() => setEditorOpen(false)}><X size={18} /></button>
        </div>
        <div className="dialog-body">
          {error && <div className="error-banner" role="alert">{error}</div>}
          <div className="form-grid">
            <div className="field">
              <label htmlFor="notify-name">渠道名称，必填</label>
              <input id="notify-name" className="input" required value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} placeholder="例如：手机 Telegram" />
            </div>
            <div className="field">
              <label htmlFor="notify-kind">渠道类型</label>
              <select id="notify-kind" className="select w-full" value={draft.kind} onChange={(e) => {
                const kind = e.target.value as NotificationChannelKind;
                setDraft({
                  ...draft,
                  kind,
                  target: kind === "bark" && !draft.target ? "https://api.day.app" : draft.target,
                });
              }}>
                {(Object.entries(channelKindLabels) as [NotificationChannelKind, string][]).map(([value, label]) => (
                  <option key={value} value={value}>{label}</option>
                ))}
              </select>
            </div>
            <div className="field span-2">
              <label htmlFor="notify-target">{meta.targetLabel}</label>
              <input id="notify-target" className="input" required={draft.kind === "telegram" || draft.kind === "ntfy"} value={draft.target} onChange={(e) => setDraft({ ...draft, target: e.target.value })} placeholder={meta.targetPlaceholder} />
            </div>
            <div className="field span-2">
              <label htmlFor="notify-secret">{meta.secretLabel}{editing?.hasSecret ? "（留空保留原值）" : meta.secretRequired ? "，必填" : ""}</label>
              <input id="notify-secret" className="input" type="password" autoComplete="off" required={meta.secretRequired && !editing?.hasSecret} value={draft.secret ?? ""} onChange={(e) => setDraft({ ...draft, secret: e.target.value })} placeholder={meta.secretPlaceholder} />
              <p className="field-help">凭据只保存在系统钥匙串，不会明文写入 SQLite。</p>
            </div>
            {meta.hasSigning && (
              <div className="field span-2">
                <label htmlFor="notify-signing">签名密钥 / HMAC Secret（可选）{editing?.hasSigningSecret ? " · 已配置" : ""}</label>
                <input id="notify-signing" className="input" type="password" autoComplete="off" value={draft.signingSecret ?? ""} onChange={(e) => setDraft({ ...draft, signingSecret: e.target.value, clearSigningSecret: false })} placeholder={editing?.hasSigningSecret ? "留空保留原值" : "飞书/钉钉加签密钥或 Webhook HMAC 密钥"} />
                {editing?.hasSigningSecret && (
                  <label className="check-row"><input type="checkbox" checked={Boolean(draft.clearSigningSecret)} onChange={(e) => setDraft({ ...draft, clearSigningSecret: e.target.checked })} />清除已保存的签名密钥</label>
                )}
              </div>
            )}
            <div className="field">
              <label htmlFor="notify-severity">最低严重级别</label>
              <select id="notify-severity" className="select w-full" value={draft.minSeverity} onChange={(e) => setDraft({ ...draft, minSeverity: e.target.value as NotificationSeverity })}>
                {(Object.entries(severityLabels) as [NotificationSeverity, string][]).map(([value, label]) => (
                  <option key={value} value={value}>{label}</option>
                ))}
              </select>
            </div>
            <div className="field">
              <label>状态</label>
              <label className="check-row mt-2"><input type="checkbox" checked={draft.enabled} onChange={(e) => setDraft({ ...draft, enabled: e.target.checked })} />保存后立即启用</label>
            </div>
            <div className="field">
              <label htmlFor="notify-quiet-start">静默开始（可选）</label>
              <input id="notify-quiet-start" className="input" type="time" value={draft.quietStart ?? ""} onChange={(e) => setDraft({ ...draft, quietStart: e.target.value })} />
            </div>
            <div className="field">
              <label htmlFor="notify-quiet-end">静默结束（可选）</label>
              <input id="notify-quiet-end" className="input" type="time" value={draft.quietEnd ?? ""} onChange={(e) => setDraft({ ...draft, quietEnd: e.target.value })} />
            </div>
            <p className="field-help span-2">静默时段内不推送「信息/警告」级通知；「严重」级故障（宕机、磁盘告警、备份失败等）会穿透静默时段送达。</p>
            <div className="field span-2">
              <span className="field-label">订阅事件（全不选表示订阅全部）</span>
              <div className="flex flex-wrap gap-3 mt-2">
                {allEventKinds.map((kind) => (
                  <label className="check-row" key={kind}>
                    <input type="checkbox" checked={draft.events.includes(kind)} onChange={() => toggleEvent(kind)} />
                    {eventLabels[kind]}
                  </label>
                ))}
              </div>
            </div>
          </div>
        </div>
        <div className="dialog-footer">
          <button className="btn" type="button" onClick={() => setEditorOpen(false)} disabled={busy !== ""}>取消</button>
          <button className="btn btn-primary" type="submit" disabled={busy !== ""}>{busy === "save" ? "保存中" : "保存渠道"}</button>
        </div>
      </form>
    </dialog>
  </section>;
}
