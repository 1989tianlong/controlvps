import { Component, type ReactNode } from "react";
import { RefreshCw, ShieldAlert } from "lucide-react";

interface State {
  error?: Error;
}

export class AppErrorBoundary extends Component<{ children: ReactNode }, State> {
  state: State = {};

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  render() {
    if (!this.state.error) return this.props.children;
    return <main className="fatal-error"><ShieldAlert size={32} /><h1>界面渲染失败</h1><p>本地数据仍保留。请重新加载界面；如果问题持续，请将以下错误反馈给开发者。</p><pre><code>{this.state.error.message}</code></pre><button className="btn btn-primary" type="button" onClick={() => window.location.reload()}><RefreshCw size={16} />重新加载</button></main>;
  }
}
