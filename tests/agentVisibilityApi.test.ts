import { expect, it } from "vitest";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { saveAgentVisibility } from "../src/features/usage/agentVisibilityApi";

it("Agent 勾选保存通过真实 invoke 使用 Rust input 参数，不在根部传递字段", async () => {
  // Tauri's mock IPC needs its window transport but no DOM.
  const previous = globalThis.window;
  Object.assign(globalThis, { window: {} });
  try {
    mockIPC((command, payload) => {
      expect(command).toBe("agent_visibility_update");
      expect(payload).toEqual({ input: { visibleAgents: ["codex", "kimi_code"] } });
      return { visibleAgents: ["codex", "kimi_code"] };
    });
    expect(await saveAgentVisibility(["codex", "kimi_code"])).toEqual({
      visibleAgents: ["codex", "kimi_code"],
    });
  } finally {
    clearMocks();
    Object.assign(globalThis, { window: previous });
  }
});
