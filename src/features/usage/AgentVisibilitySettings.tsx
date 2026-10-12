import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AGENTS, allAgentIds, normalizeVisibleAgents } from "./usageDisplay";
import { AgentGlyph } from "./usageIcons";
import { saveAgentVisibility } from "./agentVisibilityApi";
import "./usage.css";

type LoadState = "loading" | "ready" | "unavailable";

/**
 * 设置页「用量页显示的 Agent」卡片：勾选集合通过 agent_visibility_update
 * 立即保存，用量页按它裁剪选项卡、数据源与统计范围。加载失败（浏览器预览）
 * 时按全部可见展示，仍可操作并在保存失败时提示。
 */
export default function AgentVisibilitySettings() {
  // null = 尚未加载，按全部可见渲染。
  const [visible, setVisible] = useState<string[] | null>(null);
  const [state, setState] = useState<LoadState>("loading");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    invoke<{ visibleAgents: string[] }>("agent_visibility")
      .then((value) => {
        if (!active) return;
        setVisible(normalizeVisibleAgents(value.visibleAgents));
        setState("ready");
      })
      .catch(() => {
        if (active) setState("unavailable");
      });
    return () => {
      active = false;
    };
  }, []);

  const checked = new Set(visible ?? allAgentIds());
  const onlyOne = checked.size === 1;
  const toggle = async (id: string, next: boolean) => {
    const previous = visible ?? allAgentIds();
    const nextList = normalizeVisibleAgents(
      next ? [...previous, id] : previous.filter((value) => value !== id),
    );
    setVisible(nextList);
    setSaving(true);
    setError(null);
    try {
      const saved = await saveAgentVisibility(nextList);
      setVisible(normalizeVisibleAgents(saved.visibleAgents));
    } catch {
      setVisible(previous);
      setError("保存失败，原设置已保留，请重试。");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="card agent-visibility">
      <h2>用量页显示的 Agent</h2>
      <p className="muted small">
        取消勾选后，用量页不再显示对应 Agent
        的选项卡与数据源，其记录也不进入统计范围。至少保留一个。
      </p>
      <div className="agent-visibility-grid" role="group" aria-label="用量页显示的 Agent">
        {AGENTS.map(([id, label]) => {
          const isChecked = checked.has(id);
          return (
            <label key={id} className="agent-visibility-row">
              <input
                type="checkbox"
                checked={isChecked}
                disabled={saving || state === "loading" || (isChecked && onlyOne)}
                onChange={(e) => void toggle(id, e.target.checked)}
              />
              <AgentGlyph id={id} />
              <span>{label}</span>
            </label>
          );
        })}
      </div>
      <p className="muted small">更改立即保存到本机；模型筛选与菜单栏显示不受影响。</p>
      {error && (
        <p role="alert" className="monitor-error">
          {error}
        </p>
      )}
    </div>
  );
}
