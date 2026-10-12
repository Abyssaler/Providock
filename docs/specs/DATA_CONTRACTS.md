# 数据契约 v0.1

这是统计语义的唯一规范。适配器、数据库、IPC、UI 和导出必须使用同一口径。字段为设计名称，具体 Rust 类型可在搭建时调整，但不得改变含义。

## 1. 基本数值与未知值

Token 为非负整数。Rust 使用检查溢出的整数；IPC/JSON 用十进制字符串，避免超过 JavaScript 2^53 后失真。SQLite 单值可使用检查范围的 INTEGER，聚合必须处理 SUM 溢出；超出 i64 的值使用十进制文本及 Rust 整数运算，不转 REAL。UI 可显示 1.2M，详情/导出保留原值。

Money 为 `{amount: decimal_string, currency: ISO_currency_or_vendor_unit}`。金额不使用二进制浮点累计；保留源精度及单位转换规则，费用 tick 用整数转换。不同货币分开展示；不默认按汇率合并。Credits、请求数、Token 和货币是不同单位。

`null` 表示源未提供、无法可靠推导或不适用；0 只表示明确的零。每个核心字段有 `quality = reported | derived | estimated | unknown`，事件还有 `completeness = complete | partial | unknown`。未知原因至少包括 absent、unsupported、format_unknown、truncated、disconnected、permission_denied。

## 2. UsageEvent：逻辑计量事件

| 字段 | 含义 |
| --- | --- |
| schema_version | 标准契约版本，初始 1 |
| event_id | 逻辑事件稳定身份；不是 Token 数组的哈希 |
| agent | claude_code / codex / gemini_cli / grok_build / opencode / pi / mcode / kimi_code / claude_desktop |
| source_id、stream_domain | 源实例；local_log / gateway / imported_cc_switch |
| session_id、parent_session_id、actor_id、turn_id | 原始身份，不存在则 null；子 Agent 用 actor_id 区分 |
| project_ref | 本机项目引用；敏感路径单独存，展示/导出可脱敏 |
| occurred_at、observed_at | 事件发生 UTC 时间和本机观察时间；缺发生时间不伪造为扫描时间 |
| model_raw、model_normalized | 原始名称、确定性别名归一化名称，保留版本/渠道 |
| granularity | request / turn_aggregate / message / session_delta / unknown |
| request_count | 确认的模型调用次数；聚合源未提供时为 null，不能强行赋值 1 |
| tokens | 下文定义的互斥分量及附属字段 |
| cost_reported、cost_estimated | 可分别存在；带币种、质量、价格版本和范围 |
| account_link | 可空；包含 confirmed / explicit_identifier / unknown 的依据 |
| evidence_refs | 一个或多个源定位、解析器版本和字段映射引用 |

示例为合成数据，不是实际 Kimi 账单：

```json
{
  "schema_version": 1,
  "event_id": "synthetic:kimi:request-001",
  "agent": "kimi_code",
  "source_id": "synthetic-source",
  "stream_domain": "local_log",
  "session_id": "session-001",
  "actor_id": "main",
  "granularity": "request",
  "request_count": "1",
  "tokens": {
    "input_fresh": "1200",
    "input_cache_read": "800",
    "input_cache_write": "0",
    "input_total": "2000",
    "output_total": "300",
    "output_reasoning": null,
    "total": "2300"
  },
  "completeness": "complete",
  "cost_reported": null,
  "cost_estimated": null
}
```

## 3. Token 与缓存公式

标准输入分量互斥：`input_fresh` 未命中/未写缓存输入、`input_cache_read` 读取缓存输入、`input_cache_write` 创建缓存输入。来源有完整语义时：

```text
input_total = input_fresh + input_cache_read + input_cache_write
total = input_total + output_total
cache_read_ratio = sum(input_cache_read) / sum(input_total)
```

`output_reasoning` 是附属细分信息。若源 output 已包含 reasoning，不能再加；若源 output 只含可见输出，适配器在有证据时将两者相加得到 output_total。每个源声明 `output_semantics`。不能把所有来源套用同一加法。

源输入包含 cached 子集时，例如总输入 1000、cached 400，应得 fresh 600、read 400、input_total 1000，不能得 1400。Anthropic 等源可能将 fresh/read/write 分列，此时才求和。缓存创建 TTL 分量（例如 5min / 1h）是 cache_write 的子集，不能再次加到总输入。

如果已知 input_total 和 cache_read，但不知道 fresh/write 划分，可以计算读缓存占比，但未知分量保持 null；如果输入总量未知，不能通过把缺失字段补零得到“总 Token”。可另显示“已知 Token 小计”，同时给出缺失事件数。

缓存命中指标默认命名为“读缓存 Token 占比”，只在具有已知 read 与 input_total 的事件上计算，并显示参与覆盖率：适用记录数/筛选记录数、已知输入总量。全零分母显示“—”，不显示 0%。使用加权总量，不能平均各请求百分比。请求命中率仅在源确有请求粒度且 hit 状态完整时单独提供。

禁止将 context window 占用、`contextTokens`、输入字数、状态栏百分比当成消耗 Token。精确 Token 来自上游或 Agent 记录的 usage；tokenizer 只能标记为估算，并从默认精确总量分开。

## 4. 物理观察与逻辑去重

一条物理 observation 由 source_instance + file_identity + generation + offset/row_id 定位。逻辑 event 可以由多条观察支持。存在原生 request/message/operation ID 时，按其作用域生成稳定身份；消息的部分 usage 更新替换旧版本，不逐次累加。

没有原生 ID 时，使用会话、actor、协议序列/来源位置和源代数建立事件身份。数字、模型、时间组成的哈希不能单独用于去重，因为两个真实请求可能完全相同。跨文件复制/分叉只有在明确共享日志祖先、原生 ID 或可证实的顺序区间时归并；完整前缀复制可用序列指纹确认，不删除只是内容相同的独立请求。

不能确认重复的导入或网关流量留在独立 stream_domain 并显示潜在重叠，不混入默认全局合计。已确认重复记录保留证据映射而非删除来源。保留轻量身份墓碑，避免用户清理明细后旧文件再次计入。

## 5. 累计与增量

输入计数器必须声明 `counter_kind = delta | cumulative | message_snapshot | turn_aggregate` 和其作用域。

累计源按 session/actor/计数代数保存基线：同代增量为 current - previous；重复快照产生零增量；倒退需识别重置、分叉或乱序，不能简单取绝对值。模型变化与计数器重置不是同一件事。无法识别时隔离异常并标记缺口。

单次 usage 与累计 usage 同时出现时优先使用可确认的单次量，累计量用于对账，不能两份都计入。完整会话起点明确且基线为零时可导入首个累计量；截断日志的首个累计量只作未知期间基线，除非能证明其归属，否则不能全部归到扫描当天。

turn_aggregate 是独立回合计量，不做差分。例如 Grok 连续两轮都记录输入 1000，真实合计为 2000；不能误去重为 1000。

会话回退、撤销、重试和分叉可能隐藏聊天内容，但已经发生的模型消耗不会随聊天内容删除。计费账本保留原事件，活动对话链单独处理。

## 6. 费用

源明确给出的合法 cost=0 具有意义，不能用估算覆盖。源 reported cost 可能是某个聚合范围或部分计价，必须标记 complete / partial 与覆盖。默认显示已报告费用小计；有些事件只有估算时显示“估算”分项和混合范围说明，不把混合数字叫实际花费。

估算价格键包含 provider/channel、model_version、currency、effective_from/to、区域、长上下文档位、输入/输出/cache read/cache write/TTL 等差异。不可只凭模型名字选一个全球价格。未知价格不计算费用。历史价格更新不悄悄覆盖历史估值；重算保存新估值版本，允许查看采用的版本。

Coding Plan 包含消费不自动乘公开 API 价格当账单；如用户选择“API 等价估算”应单独显示，与实际订阅费用区分。余额变动也不等于本机花费，可能涉及充值、赠金、退款或其他机器。

## 7. 账户领域

`Account` 包含平台、区域、显示名、可选官方用户 ID、组织/项目 scope、配置类型和 credential_ref。`Credential` 有独立身份、类型、掩码、系统凭据库引用和能力；多个 Key 可以属于同一账户。`Wallet`、`Subscription` 和 `KeyBudget` 分开，平台可同时返回现金与计划。

没有官方账户身份时不要因为两个余额相同就自动合并钱包。用户可确认“共享钱包”；只对确认同一 wallet_id 的额度做分组。账户页不自动把多 Key 的共享余额相加。

`AccountSnapshot` 包含 snapshot_id、account_id、queried_at、provider_as_of、evidence、status、balances[]、quota_windows[]、usage_metrics[]、expires_at 以及字段质量。只保存标准化必要字段，默认不保存完整鉴权响应。

`Balance` 含 kind=paid/grant/available/credit、amount、currency、wallet_scope、expires_at、来源公式。total_balance 已含 paid/grant 时不能再相加。可用余额为负时保留其真实值，不能静默归零。

## 8. QuotaWindow

| 字段 | 定义 |
| --- | --- |
| window_id | 平台+计划+scope+模型/资源组+窗口标识，不能仅用“5h” |
| label、resource | 官方/友好名称及模型、工具或资源池 |
| scope | account / organization / project / key / subscription |
| unit | tokens / requests / credits / currency / percent / vendor_unit |
| period_kind | rolling / calendar / fixed / unknown |
| duration_seconds | 实际已知时填写；没有证据不得推断 7 天 |
| starts_at、resets_at | RFC3339 时间，未知为 null，保留源时区信息 |
| used、limit、remaining | 十进制值，可为 null；明确公式时才相减 |
| used_ratio、remaining_ratio | 0..1 的十进制比例，明确源给的是已用还是剩余 |
| raw_ratio、over_limit | 保存超限信息；可视进度条可封顶，文字不能丢掉超限 |
| quality、evidence_ref | 字段质量与来源 |

只返回已用比例时，可得 remaining_ratio=1-used_ratio，但不能反推绝对 Token 数。显示“剩余 42%”，不显示“剩余 42 Tokens”。同时存在 5h、7d、模型专属、工具调用窗口时全部保留，不能取最小数拼成一个“总剩余”。倒计时到零只显示“等待平台刷新”，不假定额度已重置。

`status` 统一为 ok / stale / refreshing / auth_required / rate_limited / unsupported / incompatible / unavailable；无支持能力和暂时联网失败不同。上次成功快照可保留，但必须带原时间与 stale 标记。

## 9. 存储与聚合

计划表：sources、source_files、ingestion_cursors、parse_runs、sessions、actors、usage_events、usage_observations、event_identity_tombstones、accounts、credential_refs、wallet_links、account_snapshots、quota_windows、usage_metrics、price_versions、settings、diagnostic_events。禁止普通表出现 credential_plaintext、prompt_body、response_body 字段。

事件按 occurred_at/source/agent/model/session 建索引；聚合结果记录活跃账本版本和时区。按本机时区分日，可切换 UTC；夏令时一天可能不是 24 小时，筛选用半开时间区间 [start,end)。没发生时间的事件单独列为“时间未知”，不塞进今天。

总体记录数命名“计量记录”，混合粒度时不叫请求数。请求数量仅对 request_count 已知部分求和并显示覆盖；Token 统计提供已知小计/完整总量状态。会话总量可包含子 Agent，展示父子关系，父汇总记录不能再当独立消耗相加。

## 2026-10-12 模型定价与 IPC 增量

ModelPrice 键为 Agent + 精确模型名 + 可选 source_id，source_id 必须属于该 Agent；精确来源优先于 Agent 通用规则，不凭同名模型跨 Agent 猜测厂商。USD/CNY 分币种，输入为每百万 Token 的 fresh/output/cache read/cache write 四项非负十进制费率。使用已有 price_versions 追加新版本及停用版本，旧行不覆盖。

当前定价估算是只读比较：以最新启用规则计算筛选事件，不修改 usage_events 中 cost_reported、cost_estimated 或 Token。互斥分量只计一次；未知数量不当零，未知费率仅在数量明确为零时不影响完整性。部分估算与完整覆盖率分别显示，金额保持精确十进制，模型统计的 Token 标为已知小计。

ModelPrice 可选两种计费策略，随 rates_json 的 `strategy` 键追加存储（tier 字符串不变，旧行无该键照常读取）：

- 分时段价格：`peak`（高峰费率组）+ `peakWindows`（`timezone` 目前仅 Asia/Shanghai，固定 UTC+8 无夏令时；`weekdaysOnly`；`ranges` 为 1–4 个 "HH:MM-HH:MM" 半开时段 [start,end)，不跨午夜、不重叠）。事件 occurred_at 换算到北京时间后，工作日（周一至周五）且本地时间落入任一时段 → 高峰费率，否则基础费率（基础费率即低谷价）。法定节假日日历未实现：所有工作日一律按高峰处理，是已知偏差。
- 长上下文升档：`longContext`（`inputOverTokens` 阈值 + 一档费率）。事件 input_total 严格大于阈值时整个请求（含输出与缓存分量）按档位费率计价；等于阈值不升档。

解析顺序：longContext 优先——配置了升档的规则不再叠加 peak（档位无高峰变体），未超阈值回到基础费率；peak 只在未配置 longContext 时参与。判定所需上下文未知时不臆造：配置了 peak 而 occurred_at 缺失/不可解析，或配置了 longContext 而 input_total 未知，该事件估算整体为未知（不是 0，也不回落基础费率）。选中档位内某分量费率为空时该分量按未知处理（部分估算）。USD/CNY 仍分币种统计。

账户列表/刷新响应的嵌套 SnapshotDto/OutcomeDto 统一 camelCase，并通过共享合成夹具验证真实 DB→IPC→前端；领域对象与原适配器样本命名保持不变。credentialReady 仅表示本地凭据是否存在，不代表平台鉴权成功。用户已明确选择明文持久化，Key 存在独立 credentials/keys.sqlite3；普通 providock.db 和 IPC 不含明文秘密。
