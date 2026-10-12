/**
 * 平台模板（添加账户用）：平台与区域 → 账户类型 → 凭据类型与最小权限说明。
 *
 * 依据 docs/specs/ACCOUNT_ADAPTERS.md：
 * - 第 2 节列出已有官方依据的优先实现项（B01–B07），平台键与
 *   `services::account_query::resolve_plan` 接受的字符串一致；
 * - 第 4 节：不同 credential_type 的平台模板只显示所需字段；
 * - 第 6 节：无对应能力的指标隐藏，编辑页提供“平台暂未提供查询能力”。
 *
 * 这里只写“凭据类型 + 最小权限说明”，不把 E3/E0 线索写成已支持：`querySupported`
 * 为 false 的平台只保留账户记录与凭据管理，卡片不显示任何余额数字。
 */

import type { CredentialType } from "../../contracts/accounts";

export interface RegionOption {
  value: string;
  label: string;
}

export interface CredentialOption {
  type: CredentialType;
  label: string;
  /** 最小权限说明：只读、不发送推理请求、不申请写权限。 */
  minimalPermission: string;
}

export interface PlatformTemplate {
  /** 与 accounts.platform 一致的平台键。 */
  key: string;
  label: string;
  regions: RegionOption[];
  credentials: CredentialOption[];
  /** 本版是否接入官方只读查询能力（false = 只做管理，不显示余额）。 */
  querySupported: boolean;
  /** 平台能力与边界的补充说明。 */
  note: string;
}

const CN_GLOBAL: RegionOption[] = [
  { value: "cn", label: "中国" },
  { value: "global", label: "国际" },
];

const GLOBAL_ONLY: RegionOption[] = [{ value: "global", label: "国际" }];

const CN_ONLY: RegionOption[] = [{ value: "cn", label: "中国" }];

export const PLATFORM_TEMPLATES: PlatformTemplate[] = [
  {
    key: "deepseek",
    label: "DeepSeek",
    regions: CN_GLOBAL,
    querySupported: true,
    note: "官方余额接口按币种分开返回，总额已含赠金与充值。",
    credentials: [
      {
        type: "api_key",
        label: "API Key（Bearer）",
        minimalPermission:
          "只需只读余额权限（GET /user/balance）；不要授予写权限，前端不发送任何推理请求。",
      },
    ],
  },
  {
    key: "openrouter",
    label: "OpenRouter",
    regions: GLOBAL_ONLY,
    querySupported: true,
    note: "推理 Key 查到的是该 Key 的用量与预算；账户 Credits 需要管理密钥。",
    credentials: [
      {
        type: "api_key",
        label: "推理 API Key",
        minimalPermission:
          "只读当前 Key 的用量与预算（GET /api/v1/key）；不授予其他 Key 或组织管理权限。",
      },
      {
        type: "management_key",
        label: "管理密钥（management key）",
        minimalPermission:
          "仅账户 Credits（GET /api/v1/credits）需要管理密钥；普通推理 Key 不保证有该权限，最小化使用。",
      },
    ],
  },
  {
    key: "novita",
    label: "Novita",
    regions: GLOBAL_ONLY,
    querySupported: true,
    note: "返回可用额、现金、授信与欠款；原始单位按官方文档换算，不照抄旧路径。",
    credentials: [
      {
        type: "api_key",
        label: "API Key（Bearer）",
        minimalPermission:
          "只读账单余额（GET /openapi/v1/billing/balance/detail）；不申请消费或管理权限。",
      },
    ],
  },
  {
    key: "glm",
    label: "GLM / 智谱",
    regions: CN_GLOBAL,
    querySupported: true,
    note: "Coding Plan 额度来自官方插件只读接口，不是现金余额；Authorization 使用原 token。",
    credentials: [
      {
        type: "api_key",
        label: "Coding Plan Token",
        minimalPermission:
          "官方插件 query-usage 使用的只读额度接口；Authorization 用原 token，不擅自加 Bearer。",
      },
    ],
  },
  {
    key: "kimi",
    label: "Kimi Code",
    regions: CN_ONLY,
    querySupported: true,
    note: "优先本机官方 Server 只读接口；Server 不在线时显示不可用，不启动推理。",
    credentials: [
      {
        type: "local_server_token",
        label: "本机 Kimi Code Server 令牌",
        minimalPermission:
          "只读本机 Server 的官方额度接口（GET /api/v1/oauth/usage）；令牌不等于厂商推理 Key。",
      },
      {
        type: "api_key",
        label: "Coding Key（直接查询）",
        minimalPermission: "官方 CLI 使用的只读 usages 通道；作为本机 Server 通道的上游来源标注。",
      },
    ],
  },
  {
    key: "codex",
    label: "Codex / ChatGPT 官方额度",
    regions: GLOBAL_ONLY,
    querySupported: true,
    note: "跟随本机 Codex 当前的 ChatGPT 登录，只读查询订阅额度。请先在官方 Codex 中登录；无需 API Key。",
    credentials: [
      {
        type: "oauth_handle",
        label: "使用本机 Codex 登录",
        minimalPermission: "官方 App Server 只读额度，不复制登录令牌、不发起模型调用。",
      },
    ],
  },
  {
    key: "other",
    label: "其他平台",
    regions: GLOBAL_ONLY,
    querySupported: false,
    note: "通用平台：只保存引用与别名，无官方查询能力时不显示任何余额数字。",
    credentials: [
      {
        type: "api_key",
        label: "API Key",
        minimalPermission: "仅本机保存引用；没有官方只读余额接口时不查询、不猜余额。",
      },
      {
        type: "ak_sk",
        label: "AK / SK",
        minimalPermission: "仅本机保存引用；需要签名调用的平台按官方文档申请最小只读权限。",
      },
    ],
  },
];

/** 按平台键取模板；未命中返回 null（不猜默认模板）。 */
export function platformTemplate(platform: string): PlatformTemplate | null {
  const key = platform.trim().toLowerCase();
  return PLATFORM_TEMPLATES.find((t) => t.key === key) ?? null;
}

/** 平台展示名：模板优先，否则回退原始键（未知平台也照实显示）。 */
export function platformLabel(platform: string): string {
  return platformTemplate(platform)?.label ?? platform;
}

/** 区域展示名。 */
export function regionLabel(region: string | null): string | null {
  if (!region) return null;
  const known = [...CN_GLOBAL, ...CN_ONLY].find((r) => r.value === region);
  if (known) return known.label;
  return region;
}
