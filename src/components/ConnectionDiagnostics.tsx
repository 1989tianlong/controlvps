import { CheckCircle2, Network, RefreshCw, XCircle } from "lucide-react";
import type { ConnectionReport } from "../types";

interface Props {
  report?: ConnectionReport;
  error?: string;
  busy: boolean;
  onRun: () => void;
}

export function ConnectionDiagnostics({ report, error, busy, onRun }: Props) {
  return <div className="panel-body">
    <div className="diagnostic-heading">
      <div><h2>连接诊断</h2><p>逐阶段检查网络、SSH 协议、主机身份和认证。诊断只读取主机名与系统信息。</p></div>
      <button className="btn btn-primary" type="button" onClick={onRun} disabled={busy} aria-busy={busy}><RefreshCw size={15} />{busy ? "正在诊断" : report ? "重新诊断" : "开始诊断"}</button>
    </div>
    {error && <div className="error-banner" role="alert">{error}</div>}
    {!report && !busy && <div className="diagnostic-empty"><Network size={28} /><p>{error ? "修正配置或确认主机身份后重新诊断。" : "配置跳板机后，可在这里确认每一跳的连接状态。"}</p></div>}
    {busy && !report && <div className="skeleton diagnostic-skeleton" aria-label="正在诊断连接" />}
    {report && <div className="diagnostic-report">
      <div className="diagnostic-summary">
        <span className={report.success ? "badge badge-ok" : "badge badge-bad"}>{report.success ? "连接正常" : "连接失败"}</span>
        <span className="measured">总计 {report.latencyMs} ms</span>
      </div>
      <p className="diagnostic-message">{report.message}</p>
      <div className="diagnostic-route" aria-label="连接路由">{report.route.map((hop, index) => <span key={hop}>{index > 0 && <span aria-hidden="true">→</span>}<code>{hop}</code></span>)}</div>
      <ol className="diagnostic-steps">
        {report.steps.map((step, index) => <li key={`${step.name}-${index}`} className={step.success ? "is-success" : "is-failed"}>
          {step.success ? <CheckCircle2 size={18} /> : <XCircle size={18} />}
          <div><strong>{step.name}</strong><p>{step.detail}</p></div>
          <span className="measured">{step.durationMs} ms</span>
        </li>)}
      </ol>
      {report.success && <dl className="facts">
        <div><dt>主机名</dt><dd>{report.hostname || "未知"}</dd></div>
        <div><dt>操作系统</dt><dd>{report.os || "未知"}</dd></div>
        <div><dt>目标主机指纹</dt><dd className="fingerprint-text">{report.fingerprint}</dd></div>
      </dl>}
    </div>}
  </div>;
}
