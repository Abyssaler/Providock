import { invoke } from "@tauri-apps/api/core";

export interface AgentVisibility {
  visibleAgents: string[];
}

export function saveAgentVisibility(visibleAgents: string[]): Promise<AgentVisibility> {
  return invoke<AgentVisibility>("agent_visibility_update", { input: { visibleAgents } });
}
