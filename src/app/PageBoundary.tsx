import { Component, type ErrorInfo, type ReactNode } from "react";
/** Keep navigation usable if a page fails to render; never expose raw errors or account data. */
export default class PageBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() {
    return { failed: true };
  }
  componentDidCatch(_error: Error, _info: ErrorInfo) {
    /* Do not log potentially sensitive payloads. */
  }
  render() {
    if (this.state.failed)
      return (
        <div className="card" role="alert">
          <h2>页面暂时无法显示</h2>
          <p className="muted">本地数据仍然保留。可以重试或切换到其他页面。</p>
          <button onClick={() => this.setState({ failed: false })}>重新加载页面</button>
        </div>
      );
    return this.props.children;
  }
}
