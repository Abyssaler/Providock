import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import AgentVisibilitySettings from "../src/features/usage/AgentVisibilitySettings";
import UsagePage from "../src/features/usage/UsagePage";
import {
  agentsFilter,
  allAgentIds,
  isAgentVisible,
  normalizeVisibleAgents,
  visibleAgentTabs,
} from "../src/features/usage/usageDisplay";

describe("Agent 可见性纯逻辑", () => {
  it("缺省、空集合、未知 id 一律归一到全部可见，顺序随 AGENTS 且去重", () => {
    const all = allAgentIds();
    expect(all).toHaveLength(8);
    expect(normalizeVisibleAgents(null)).toEqual(all);
    expect(normalizeVisibleAgents(undefined)).toEqual(all);
    expect(normalizeVisibleAgents([])).toEqual(all);
    expect(normalizeVisibleAgents(["mcode", "unknown_agent"])).toEqual(["mcode"]);
    expect(normalizeVisibleAgents(["kimi_code", "mcode", "kimi_code"])).toEqual([
      "mcode",
      "kimi_code",
    ]);
  });
  it("agentsFilter：全部可见时省略字段，子集时生成限定列表", () => {
    expect(agentsFilter(null)).toBeUndefined();
    expect(agentsFilter(allAgentIds())).toBeUndefined();
    expect(agentsFilter(["claude_code", "codex"])).toEqual(["claude_code", "codex"]);
    expect(agentsFilter(["mcode"])).toEqual(["mcode"]);
  });
  it("选项卡与可见判断只包含可见 Agent", () => {
    const tabs = visibleAgentTabs(["mcode"]);
    expect(tabs).toHaveLength(1);
    expect(tabs[0][0]).toBe("mcode");
    expect(tabs[0][1]).toBe("MCode");
    expect(visibleAgentTabs(null)).toHaveLength(8);
    expect(isAgentVisible(["mcode", "pi"], "pi")).toBe(true);
    expect(isAgentVisible(["mcode"], "codex")).toBe(false);
    expect(isAgentVisible(null, "codex")).toBe(true);
  });
});

/**
 * 渲染冒烟：无 Tauri 运行时（浏览器/服务端渲染）也能挂载；
 * 设置卡片默认八个全选，用量页初始渲染全部选项卡与「正在读取统计…」。
 */
describe("可见性相关渲染冒烟", () => {
  it("设置卡片默认八个全选，并说明至少保留一个", () => {
    const html = renderToStaticMarkup(<AgentVisibilitySettings />);
    expect(html).toContain("用量页显示的 Agent");
    expect((html.match(/type="checkbox"/g) ?? []).length).toBe(8);
    expect((html.match(/checked=""/g) ?? []).length).toBe(8);
    expect(html).toContain("MCode");
    expect(html).toContain("Kimi Code");
    expect(html).toContain("至少保留一个");
    expect(html).not.toContain("alert");
  });
  it("用量页初始渲染保留「全部」与全部 Agent 选项卡", () => {
    const html = renderToStaticMarkup(<UsagePage />);
    expect(html).toContain("使用统计");
    expect(html).toContain("全部");
    expect(html).toContain("Claude Code");
    expect(html).toContain("Gemini CLI");
    expect(html).toContain("数据源筛选");
    expect(html).toContain("模型筛选");
    expect(html).toContain("正在读取统计…");
    expect(html).toContain("立即同步");
    // 「全部」+ 8 个 Agent 选项卡
    expect((html.match(/aria-pressed/g) ?? []).length).toBe(9);
  });
});
