//! Explicit IPC boundary: domain/fixture serialization remains snake_case;
//! every nested frontend object uses camelCase.
use crate::domain::Quality;
use crate::services::{account_query::SnapshotOutcome, accounts as acc};
use serde::Serialize;

macro_rules! fields_dto {
    ($name:ident, $source:ty, {$($field:ident: $ty:ty),* $(,)?}) => {
        #[derive(Debug, Serialize)]
        #[serde(rename_all="camelCase")]
        pub struct $name {$(pub $field: $ty),*}
        impl From<$source> for $name {
            fn from(v: $source) -> Self {Self {$($field:v.$field),*}}
        }
    };
}
fields_dto!(BalanceDto,acc::BalanceEntry,{
    kind:acc::BalanceKind,amount:String,currency:String,wallet_scope:Option<String>,expires_at:Option<String>,formula:Option<String>,quality:Quality
});
fields_dto!(QuotaDto,acc::QuotaWindow,{
    window_id:String,label:Option<String>,resource:Option<String>,scope:Option<String>,unit:acc::QuotaUnit,period_kind:Option<acc::PeriodKind>,duration_seconds:Option<u64>,starts_at:Option<String>,resets_at:Option<String>,used:Option<String>,limit:Option<String>,remaining:Option<String>,used_ratio:Option<String>,remaining_ratio:Option<String>,raw_ratio:Option<String>,over_limit:Option<bool>,quality:Quality,evidence_ref:Option<String>
});
fields_dto!(MetricDto,acc::UsageMetric,{
    name:String,value:String,unit:acc::QuotaUnit,scope:Option<String>,quality:Quality
});
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotDto {
    pub snapshot_id: i64,
    pub account_id: String,
    pub credential_id: Option<String>,
    pub queried_at: String,
    pub provider_as_of: Option<String>,
    pub status: acc::SnapshotStatus,
    pub evidence: Option<String>,
    pub error_code: Option<String>,
    pub expires_at: Option<String>,
    pub balances: Vec<BalanceDto>,
    pub quota_windows: Vec<QuotaDto>,
    pub usage_metrics: Vec<MetricDto>,
}
impl From<acc::Snapshot> for SnapshotDto {
    fn from(v: acc::Snapshot) -> Self {
        Self {
            snapshot_id: v.snapshot_id,
            account_id: v.account_id,
            credential_id: v.credential_id,
            queried_at: v.queried_at,
            provider_as_of: v.provider_as_of,
            status: v.status,
            evidence: v.evidence,
            error_code: v.error_code,
            expires_at: v.expires_at,
            balances: v.balances.into_iter().map(Into::into).collect(),
            quota_windows: v.quota_windows.into_iter().map(Into::into).collect(),
            usage_metrics: v.usage_metrics.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutcomeDto {
    pub account_id: String,
    pub status: acc::SnapshotStatus,
    pub snapshot_id: Option<i64>,
    pub queried_at: String,
    pub provider_as_of: Option<String>,
    pub evidence: Option<String>,
    pub error_code: Option<String>,
    pub balances: Vec<BalanceDto>,
    pub quota_windows: Vec<QuotaDto>,
    pub usage_metrics: Vec<MetricDto>,
    pub stale: bool,
}
impl From<SnapshotOutcome> for OutcomeDto {
    fn from(v: SnapshotOutcome) -> Self {
        Self {
            account_id: v.account_id,
            status: v.status,
            snapshot_id: v.snapshot_id,
            queried_at: v.queried_at,
            provider_as_of: v.provider_as_of,
            evidence: v.evidence,
            error_code: v.error_code,
            balances: v.balances.into_iter().map(Into::into).collect(),
            quota_windows: v.quota_windows.into_iter().map(Into::into).collect(),
            usage_metrics: v.usage_metrics.into_iter().map(Into::into).collect(),
            stale: v.stale,
        }
    }
}
