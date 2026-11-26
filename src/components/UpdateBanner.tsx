import { Download, RotateCcw, X } from "lucide-react";
import type { UpdateState } from "../hooks/useAppUpdate";

interface Props {
  state: UpdateState;
  dismissed: boolean;
  onInstall: () => void;
  onRestart: () => void;
  onDismiss: () => void;
}

export function UpdateBanner({ state, dismissed, onInstall, onRestart, onDismiss }: Props) {
  if (dismissed) return null;
  if (state.status === "error") {
    return <div className="update-banner" role="alert">
      <Download size={17} />
      <div className="update-banner-body"><strong>更新失败</strong><p>{state.message}</p></div>
      <button className="btn" type="button" onClick={onDismiss}>关闭</button>
    </div>;
  }
  if (state.status === "available") {
    return <div className="update-banner" role="status">
      <Download size={17} />
      <div className="update-banner-body">
        <strong>发现新版本 v{state.version}</strong>
        {state.notes && <p className="update-notes">{state.notes}</p>}
      </div>
      <div className="update-banner-actions">
        <button className="btn" type="button" onClick={onDismiss}>稍后</button>
        <button className="btn btn-primary" type="button" onClick={onInstall}>下载并安装</button>
      </div>
    </div>;
  }
  if (state.status === "downloading") {
    return <div className="update-banner" role="status">
      <Download size={17} />
      <div className="update-banner-body">
        <strong>正在下载 v{state.version}{state.percent === null ? "" : ` · ${state.percent}%`}</strong>
        <progress className="update-progress" max={100} value={state.percent ?? 0} />
      </div>
    </div>;
  }
  if (state.status === "ready") {
    return <div className="update-banner" role="status">
      <RotateCcw size={17} />
      <div className="update-banner-body"><strong>v{state.version} 已就绪</strong><p>重启应用后生效，当前连接和终端会关闭。</p></div>
      <div className="update-banner-actions">
        <button className="btn" type="button" onClick={onDismiss}><X size={15} />稍后</button>
        <button className="btn btn-primary" type="button" onClick={onRestart}>重启应用</button>
      </div>
    </div>;
  }
  return null;
}
