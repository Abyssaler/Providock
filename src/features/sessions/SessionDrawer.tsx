import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ContentBlock, ContentPage, SessionItem } from "../../contracts/sessions";
import {
  agentLabel,
  contentReasonLabel,
  parseContentError,
  roleLabel,
  splitCodeSegments,
  splitLinks,
} from "./contentView";

interface Props {
  session: SessionItem;
  hidePaths: boolean;
  onClose: () => void;
}

const PAGE_LIMIT = 50;

/** 正文纯文本渲染：代码块 <pre>、段落与链接域名提示；不使用 dangerouslySetInnerHTML。 */
function renderPlainText(text: string) {
  return splitCodeSegments(text).map((segment, index) => {
    if (segment.kind === "code") {
      return (
        <pre key={index} className="sess-code">
          {segment.value}
        </pre>
      );
    }
    return splitLinks(segment.value).map((part, partIndex) => {
      if (part.kind === "link" && part.domain) {
        // 不自动加载、不生成可点击元素：只展示目标域名，由用户之后决定是否打开。
        return (
          <span key={partIndex} className="sess-link" title={part.value}>
            ↗ {part.domain}
          </span>
        );
      }
      return <span key={partIndex}>{part.value}</span>;
    });
  });
}

function BlockView({ block }: { block: ContentBlock }) {
  const body = block.text === null ? null : renderPlainText(block.text);
  const label = (
    <span className="sess-role">
      {roleLabel(block.role)}
      {block.toolName && <span className="muted small"> · {block.toolName}</span>}
      {block.time && <span className="muted small"> · {block.time}</span>}
      {block.truncated && <span className="sess-warn small"> · 已截断</span>}
    </span>
  );

  if (block.isCollapsibleTool) {
    return (
      <details className="sess-block sess-tool">
        <summary>{label}</summary>
        {body ?? <p className="muted small">该工具块没有可展示的文本。</p>}
      </details>
    );
  }
  return (
    <div className="sess-block">
      {label}
      {body ?? <p className="muted small">该记录没有可展示的文本。</p>}
    </div>
  );
}

export default function SessionDrawer({ session, hidePaths, onClose }: Props) {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [content, setContent] = useState<ContentPage | null>(null);
  const [error, setError] = useState<{ code: string; message: string } | null>(null);
  const [loading, setLoading] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);

  // Escape 关闭并由父组件把焦点还给触发按钮。
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    panelRef.current?.focus();
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  useEffect(() => {
    let cancelled = false;
    invoke<boolean>("content_view_status")
      .then((value) => {
        if (!cancelled) setEnabled(value);
      })
      .catch(() => {
        if (!cancelled) setEnabled(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const load = useCallback(
    (cursor: string | null) => {
      setLoading(true);
      setError(null);
      invoke<ContentPage>("session_content_page", {
        sessionRef: {
          sourceId: session.sourceId,
          sessionId: session.sessionId,
        },
        cursor: cursor === null ? null : Number(cursor),
        limit: PAGE_LIMIT,
      })
        .then((page) => {
          setContent((prev) =>
            cursor === null || prev === null
              ? page
              : { ...page, blocks: [...prev.blocks, ...page.blocks] },
          );
        })
        .catch((e) => setError(parseContentError(e)))
        .finally(() => setLoading(false));
    },
    [session.sourceId, session.sessionId],
  );

  const enableAndLoad = () => {
    setError(null);
    setLoading(true);
    invoke<boolean>("content_view_set", { enabled: true })
      .then(() => {
        setEnabled(true);
        setLoading(false);
        load(null);
      })
      .catch((e) => {
        setError(parseContentError(e));
        setLoading(false);
      });
  };

  return (
    <div className="sess-overlay" role="presentation" onClick={onClose}>
      <div
        className="sess-drawer"
        role="dialog"
        aria-modal="true"
        aria-label={`会话详情 ${session.sessionId}`}
        ref={panelRef}
        tabIndex={-1}
        onClick={(event) => event.stopPropagation()}
      >
        <div className="sess-drawer-head">
          <div>
            <h2>
              {hidePaths && session.title
                ? "（标题已隐藏）"
                : (session.title ?? "（源未提供标题）")}
            </h2>
            <p className="muted small">
              {agentLabel(session.agent)} · 会话 {session.sessionId}
              {session.hasParent && ` · 子会话（父 ${session.parentSessionId ?? "未知"}）`}
              {session.hasSubagents && " · 含子 Agent"}
            </p>
          </div>
          <button onClick={onClose}>关闭</button>
        </div>

        <div className="sess-drawer-body">
          <section className="sess-stats" aria-label="会话统计">
            <div className="stat">
              <div className="muted small">Token 小计</div>
              <div className="num">{session.tokenTotal ?? "—"}</div>
              <div className="small muted">
                已知总量来自 {session.tokenKnownEvents}/{session.eventCount} 条计量记录
              </div>
            </div>
            <div className="stat">
              <div className="muted small">计量记录</div>
              <div className="num">{session.eventCount}</div>
              <div className="small muted">混合粒度时按记录计，不叫请求数</div>
            </div>
            <div className="stat">
              <div className="muted small">最后活跃</div>
              <div className="num small">{session.lastActiveAt ?? "时间未知"}</div>
              <div className="small muted">未知不并入今天</div>
            </div>
          </section>

          <section aria-label="来源定位">
            <h3>来源定位</h3>
            <ul className="sess-meta">
              <li>
                数据源：<code>{session.sourceId}</code>
              </li>
              <li>
                会话 ID：<code>{session.sessionId}</code>
              </li>
              <li>
                模型：
                <code>{session.models.length > 0 ? session.models.join("、") : "源未提供"}</code>
              </li>
              {content?.locatedPath && (
                <li>
                  源文件定位（源根内）：
                  <code>{hidePaths ? "（路径已隐藏）" : content.locatedPath}</code>
                </li>
              )}
              {session.projectRef && (
                <li>
                  项目：
                  <code>{hidePaths ? "（路径已隐藏）" : session.projectRef}</code>
                </li>
              )}
            </ul>
          </section>

          <section aria-label="对话正文">
            <div className="sess-content-head">
              <h3>对话正文</h3>
              {enabled && <span className="badge">按需读取 · 不落库</span>}
            </div>

            {enabled === null && <p className="muted small">读取正文开关状态中…</p>}

            {enabled === false && (
              <div className="sess-notice">
                <p>
                  <strong>对话查看未开启。</strong>
                </p>
                <p className="muted small">
                  默认不读取也不保存对话正文。开启后只读取这一个源/会话的文本，分页返回，
                  关闭抽屉即释放；统计仍来自本地索引。
                </p>
                <button className="primary" disabled={loading} onClick={enableAndLoad}>
                  开启并查看此会话
                </button>
              </div>
            )}

            {enabled === true && content === null && !loading && (
              <div className="sess-notice">
                <p className="muted small">
                  正文不自动加载。点击下方按钮才会从源文件按需读取该会话。
                </p>
                <button className="primary" onClick={() => load(null)}>
                  读取此会话正文
                </button>
              </div>
            )}

            {loading && <p className="muted small">读取中…</p>}

            {error && (
              <div className="sess-notice" role="alert">
                <p>
                  <strong>{contentReasonLabel(error.code)}</strong>
                </p>
                <p className="muted small">{error.message}</p>
              </div>
            )}

            {content !== null && (
              <>
                {content.blocks.length === 0 ? (
                  <p className="muted small">
                    该文件里没有可展示的对话文本（统计不受影响）。
                    {content.diagnostics.length > 0 && ` 诊断：${content.diagnostics.join("；")}`}
                  </p>
                ) : (
                  <div className="sess-blocks">
                    {content.blocks.map((block, index) => (
                      <BlockView block={block} key={`${index}-${block.role}`} />
                    ))}
                  </div>
                )}
                {content.hasMore && (
                  <button
                    disabled={loading}
                    onClick={() => load(content.nextCursor)}
                    style={{ marginTop: 12 }}
                  >
                    加载更多
                  </button>
                )}
              </>
            )}
          </section>
        </div>
      </div>
    </div>
  );
}
