/**
 * Kimi Code 预设供应商配置模板
 *
 * settingsConfig 结构与后端约定（后端据此写 Kimi Code 的 TOML 配置）：
 * {
 *   "type": "kimi" | "anthropic" | "openai" | "openai_responses" | "google-genai" | "vertexai",
 *   "base_url": "...",
 *   "api_key": "...",
 *   "models": {
 *     "<alias>": {
 *       "model": "<真实模型 ID>",
 *       "max_context_size": 262144,
 *       "display_name"?: "...",
 *       "capabilities"?: [...],
 *       "support_efforts"?: [...],
 *       "default_effort"?: "..."
 *     }
 *   }
 * }
 */
import type { ProviderCategory } from "../types";
import type { PresetTheme } from "./claudeProviderPresets";

export const KIMI_CODE_PROVIDER_TYPES = [
  "kimi",
  "anthropic",
  "openai",
  "openai_responses",
  "google-genai",
  "vertexai",
] as const;

export type KimiCodeProviderType = (typeof KIMI_CODE_PROVIDER_TYPES)[number];

export const KIMI_CODE_CAPABILITIES = [
  "thinking",
  "always_thinking",
  "image_in",
  "video_in",
  "audio_in",
  "tool_use",
  "dynamically_loaded_tools",
] as const;

export type KimiCodeCapability = (typeof KIMI_CODE_CAPABILITIES)[number];

export interface KimiCodeModelConfig {
  model: string;
  max_context_size: number;
  display_name?: string;
  capabilities?: string[];
  support_efforts?: string[];
  default_effort?: string;
}

export interface KimiCodeSettingsConfig {
  type: KimiCodeProviderType;
  base_url: string;
  api_key: string;
  api_key_env?: string;
  custom_headers?: Record<string, string>;
  default_model?: string;
  models: Record<string, KimiCodeModelConfig>;
}

export interface KimiCodeProviderPreset {
  name: string;
  nameKey?: string;
  websiteUrl: string;
  apiKeyUrl?: string;
  settingsConfig: KimiCodeSettingsConfig;
  isOfficial?: boolean;
  isPartner?: boolean;
  partnerPromotionKey?: string;
  category?: ProviderCategory;
  primePartner?: boolean;
  theme?: PresetTheme;
  endpointCandidates?: string[];
  icon?: string;
  iconColor?: string;
}

export const KIMI_CODE_DEFAULT_CONFIG: KimiCodeSettingsConfig = {
  type: "kimi",
  base_url: "",
  api_key: "",
  models: {},
};

export const kimiCodeProviderPresets: KimiCodeProviderPreset[] = [
  {
    name: "Kimi Official",
    websiteUrl: "https://platform.moonshot.ai",
    apiKeyUrl: "https://platform.moonshot.ai/console/api-keys",
    settingsConfig: {
      type: "kimi",
      base_url: "https://api.moonshot.ai/v1",
      api_key: "",
      models: {
        "kimi-for-coding": {
          model: "kimi-for-coding",
          max_context_size: 262144,
        },
        k3: {
          model: "k3",
          max_context_size: 1048576,
        },
      },
    },
    isOfficial: true,
    category: "official",
    endpointCandidates: ["https://api.moonshot.ai/v1"],
    icon: "kimi",
    iconColor: "#1783FF",
  },
  {
    name: "Anthropic Compatible",
    websiteUrl: "https://www.anthropic.com",
    settingsConfig: {
      type: "anthropic",
      base_url: "https://api.anthropic.com",
      api_key: "",
      models: {
        sonnet: {
          model: "claude-sonnet-4-5",
          max_context_size: 200000,
        },
      },
    },
    category: "third_party",
    icon: "anthropic",
  },
];
