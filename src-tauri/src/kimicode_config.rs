//! Kimi Code CLI live configuration (`$KIMI_CODE_HOME/config.toml`, TOML).
//!
//! Switch-mode app: switching a provider writes `[providers."<id>"]` plus its
//! `[models."<alias>"]` entries into the live file and points `default_model`
//! at the provider's default alias. Everything unrelated to providers/models
//! (`permission`, `hooks`, `thinking`, comments, ...) is preserved via
//! `toml_edit`; entries this app previously managed are removed when the
//! provider is switched away from or deleted, while user-owned entries stay.
use crate::config::{atomic_write_private, get_home_dir};
use crate::error::AppError;
use serde_json::Value;
use std::{fs, path::PathBuf};
use toml_edit::{DocumentMut, Item, Table};

/// Recognized Kimi Code provider types (from the official documentation).
const PROVIDER_TYPES: [&str; 6] = [
    "kimi",
    "anthropic",
    "openai",
    "openai_responses",
    "google-genai",
    "vertexai",
];

/// Provider-level fields written into `[providers."<id>"]`. Any other
/// JSON key in the provider config is passed through unchanged.
const PROVIDER_PASSTHROUGH_KEYS: [&str; 2] = ["custom_headers", "env"];

/// Kimi Code data directory: settings override > `$KIMI_CODE_HOME` > `~/.kimi-code`.
pub(crate) fn data_dir() -> PathBuf {
    if let Some(custom) = crate::settings::get_kimicode_override_dir() {
        return custom;
    }
    if let Ok(value) = std::env::var("KIMI_CODE_HOME") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    get_home_dir().join(".kimi-code")
}

pub(crate) fn config_path() -> PathBuf {
    data_dir().join("config.toml")
}

pub(crate) fn read_config(path: &std::path::Path) -> Result<DocumentMut, AppError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(DocumentMut::new()),
        Err(e) => return Err(AppError::io(path, e)),
    };
    text.parse::<DocumentMut>().map_err(|e| {
        AppError::localized(
            "kimicode.config.invalid_toml",
            format!("Kimi Code config.toml 格式错误: {e}"),
            format!("Invalid Kimi Code config.toml: {e}"),
        )
    })
}

fn write_config(path: &std::path::Path, doc: &DocumentMut) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }
    atomic_write_private(path, doc.to_string().as_bytes())
}

fn json_to_toml_value(value: &Value) -> Option<toml_edit::Value> {
    match value {
        Value::String(s) => Some(toml_edit::Value::from(s.as_str())),
        Value::Bool(b) => Some(toml_edit::Value::from(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(toml_edit::Value::from(i))
            } else {
                n.as_f64().map(toml_edit::Value::from)
            }
        }
        Value::Array(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(json_to_toml_value(item)?);
            }
            Some(toml_edit::Value::Array(array))
        }
        Value::Object(map) => {
            let mut table = toml_edit::InlineTable::new();
            for (key, item) in map {
                table.insert(key, json_to_toml_value(item)?);
            }
            Some(toml_edit::Value::InlineTable(table))
        }
        Value::Null => None,
    }
}

/// Insert a JSON object as a TOML table, keeping only the whitelisted
/// passthrough tables plus scalar/array fields.
fn json_object_to_table(config: &Value, extra_table_keys: &[&str]) -> Result<Table, AppError> {
    let object = config
        .as_object()
        .ok_or_else(|| AppError::Config("Kimi Code provider entry must be an object".into()))?;
    let mut table = Table::new();
    for (key, value) in object {
        let is_passthrough_table = extra_table_keys.contains(&key.as_str());
        match value {
            Value::Object(_) if is_passthrough_table => {
                let Some(toml_edit::Value::InlineTable(inline)) = json_to_toml_value(value) else {
                    continue;
                };
                let mut sub = Table::new();
                for (sub_key, sub_value) in inline.iter() {
                    sub.insert(sub_key, Item::Value(sub_value.clone()));
                }
                table.insert(key, Item::Table(sub));
            }
            Value::Object(_) | Value::Null => {}
            _ => {
                if let Some(v) = json_to_toml_value(value) {
                    table.insert(key, Item::Value(v));
                }
            }
        }
    }
    Ok(table)
}

pub(crate) fn validate_provider(id: &str, config: &Value) -> Result<(), AppError> {
    if id.is_empty()
        || id
            .chars()
            .any(|c| !c.is_ascii_alphanumeric() && !matches!(c, '-' | '_' | '.' | '/' | ':'))
    {
        return Err(AppError::InvalidInput(
            "Kimi Code provider key must contain letters, digits, '-', '_', '.', '/' or ':'".into(),
        ));
    }
    let object = config.as_object().ok_or_else(|| {
        AppError::InvalidInput("Kimi Code provider config must be an object".into())
    })?;
    match object.get("type").and_then(Value::as_str) {
        Some(t) if PROVIDER_TYPES.contains(&t) => {}
        _ => {
            return Err(AppError::InvalidInput(format!(
                "Kimi Code provider type must be one of: {}",
                PROVIDER_TYPES.join(", ")
            )))
        }
    }
    if let Some(base_url) = object.get("base_url").and_then(Value::as_str) {
        if !url::Url::parse(base_url)
            .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
        {
            return Err(AppError::InvalidInput(
                "Enter a valid Kimi Code base URL".into(),
            ));
        }
    }
    let api_key = object.get("api_key").and_then(Value::as_str).unwrap_or("");
    let api_key_env = object
        .get("api_key_env")
        .and_then(Value::as_str)
        .unwrap_or("");
    if api_key.trim().is_empty() && api_key_env.trim().is_empty() {
        return Err(AppError::InvalidInput(
            "Enter an API key or an api_key_env variable name".into(),
        ));
    }
    let models = object
        .get("models")
        .and_then(Value::as_object)
        .filter(|models| !models.is_empty())
        .ok_or_else(|| AppError::InvalidInput("Add at least one Kimi Code model".into()))?;
    for (alias, model) in models {
        if alias.trim().is_empty() {
            return Err(AppError::InvalidInput(
                "Kimi Code model alias must not be empty".into(),
            ));
        }
        let entry = model.as_object().ok_or_else(|| {
            AppError::InvalidInput(format!("Kimi Code model '{alias}' must be an object"))
        })?;
        if entry
            .get("model")
            .and_then(Value::as_str)
            .is_none_or(|m| m.trim().is_empty())
        {
            return Err(AppError::InvalidInput(format!(
                "Kimi Code model '{alias}' is missing the model id"
            )));
        }
        match entry.get("max_context_size").and_then(Value::as_i64) {
            Some(size) if size >= 1 => {}
            _ => {
                return Err(AppError::InvalidInput(format!(
                    "Kimi Code model '{alias}' requires a positive integer max_context_size"
                )))
            }
        }
    }
    Ok(())
}

/// Provider id referenced by the current `default_model`, if resolvable.
fn provider_of_alias<'a>(doc: &'a DocumentMut, alias: &str) -> Option<&'a str> {
    doc.get("models")?
        .as_table_like()?
        .get(alias)?
        .as_table_like()?
        .get("provider")?
        .as_str()
}

fn model_aliases_of_provider(doc: &DocumentMut, provider_id: &str) -> Vec<String> {
    doc.get("models")
        .and_then(Item::as_table_like)
        .map(|models| {
            models
                .iter()
                .filter(|(_, item)| {
                    item.as_table_like()
                        .and_then(|t| t.get("provider"))
                        .and_then(Item::as_str)
                        == Some(provider_id)
                })
                .map(|(alias, _)| alias.to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Remove a provider projection: its `[providers."<id>"]` table, every
/// `[models."<alias>"]` entry bound to it, and a dangling `default_model`.
fn remove_projection(doc: &mut DocumentMut, id: &str) {
    let aliases = model_aliases_of_provider(doc, id);
    if let Some(models) = doc.get_mut("models").and_then(Item::as_table_like_mut) {
        for alias in &aliases {
            models.remove(alias);
        }
        if models.is_empty() {
            doc.remove("models");
        }
    }
    if let Some(providers) = doc.get_mut("providers").and_then(Item::as_table_like_mut) {
        providers.remove(id);
        if providers.is_empty() {
            doc.remove("providers");
        }
    }
    let dangling = doc
        .get("default_model")
        .and_then(Item::as_str)
        .is_some_and(|alias| aliases.iter().any(|a| a == alias));
    if dangling {
        doc.remove("default_model");
    }
}

/// Project a provider into the document: provider table, model tables, and
/// `default_model` (the configured `default_model` alias, else the first one).
fn apply_projection(doc: &mut DocumentMut, id: &str, config: &Value) -> Result<(), AppError> {
    let object = config.as_object().expect("validated object");
    let models = object
        .get("models")
        .and_then(Value::as_object)
        .expect("validated models");

    let provider_table = json_object_to_table(
        &Value::Object(
            object
                .iter()
                .filter(|(key, _)| !matches!(key.as_str(), "models" | "default_model" | "config"))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
        &PROVIDER_PASSTHROUGH_KEYS,
    )?;

    if !doc.contains_key("providers") {
        doc["providers"] = Item::Table(Table::new());
    }
    let providers = doc["providers"]
        .as_table_mut()
        .ok_or_else(|| AppError::Config("Kimi Code providers must be a table".into()))?;
    providers.insert(id, Item::Table(provider_table));

    if !doc.contains_key("models") {
        doc["models"] = Item::Table(Table::new());
    }
    let models_table = doc["models"]
        .as_table_mut()
        .ok_or_else(|| AppError::Config("Kimi Code models must be a table".into()))?;

    let mut first_alias: Option<String> = None;
    for (alias, model) in models {
        let entry = model.as_object().expect("validated model");
        let model_table = json_object_to_table(
            &Value::Object(
                entry
                    .iter()
                    .filter(|(key, _)| key.as_str() != "provider")
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            ),
            &[],
        )?;
        if model_table.get("model").is_none() || model_table.get("max_context_size").is_none() {
            return Err(AppError::Config(format!(
                "Kimi Code model '{alias}' requires model and max_context_size"
            )));
        }
        // `provider` is written first so the routing key stays on top.
        let mut ordered = Table::new();
        ordered.insert("provider", toml_edit::value(id));
        for (key, value) in model_table.iter() {
            ordered.insert(key, value.clone());
        }
        models_table.insert(alias, Item::Table(ordered));
        if first_alias.is_none() {
            first_alias = Some(alias.clone());
        }
    }

    let default_alias = object
        .get("default_model")
        .and_then(Value::as_str)
        .filter(|alias| models.contains_key(*alias))
        .map(str::to_string)
        .or(first_alias)
        .expect("validated models non-empty");
    doc["default_model"] = toml_edit::value(default_alias);
    Ok(())
}

/// Write the live projection for a switch: drop stale managed providers,
/// merge the provider's raw `config` TOML extras, then write its entries.
pub(crate) fn write_live_provider(
    id: &str,
    config: &Value,
    stale_provider_ids: &[String],
) -> Result<(), AppError> {
    validate_provider(id, config)?;
    let path = config_path();
    let mut doc = read_config(&path)?;
    for stale_id in stale_provider_ids {
        if stale_id != id {
            remove_projection(&mut doc, stale_id);
        }
    }
    if let Some(extra) = config.get("config").and_then(Value::as_str) {
        merge_toml_snippet(&mut doc, extra)?;
    }
    apply_projection(&mut doc, id, config)?;
    write_config(&path, &doc)
}

/// Insert a provider projection without removing others (import/sync helper).
#[allow(dead_code)]
pub(crate) fn set_provider(id: &str, config: &Value) -> Result<(), AppError> {
    validate_provider(id, config)?;
    let path = config_path();
    let mut doc = read_config(&path)?;
    if let Some(extra) = config.get("config").and_then(Value::as_str) {
        merge_toml_snippet(&mut doc, extra)?;
    }
    apply_projection(&mut doc, id, config)?;
    write_config(&path, &doc)
}

pub(crate) fn remove_provider(id: &str) -> Result<(), AppError> {
    let path = config_path();
    if !path.exists() {
        return Ok(());
    }
    let mut doc = read_config(&path)?;
    remove_projection(&mut doc, id);
    write_config(&path, &doc)
}

/// Merge a raw TOML snippet (the provider's `config` field / common config
/// snippet) into the document. Provider-owned sections are refused so the
/// projection stays the single writer of `providers` / `models` /
/// `default_model`.
fn merge_toml_snippet(doc: &mut DocumentMut, snippet: &str) -> Result<(), AppError> {
    let trimmed = snippet.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let snippet_doc = trimmed.parse::<DocumentMut>().map_err(|e| {
        AppError::localized(
            "kimicode.snippet.invalid_toml",
            format!("Kimi Code 通用配置片段 TOML 格式错误: {e}"),
            format!("Invalid Kimi Code TOML snippet: {e}"),
        )
    })?;
    for (key, item) in snippet_doc.iter() {
        if matches!(key, "providers" | "models" | "default_model") {
            return Err(AppError::InvalidInput(format!(
                "Kimi Code common config must not touch '{key}' (managed per provider)"
            )));
        }
        doc[key] = item.clone();
    }
    Ok(())
}

/// Reconstruct the current provider's settings JSON from the live file.
///
/// Returns `Err` when the file is missing (mirrors Claude's read semantics);
/// returns `Ok(Value::Null)`-free empty object when no default model is set.
pub(crate) fn read_live_provider_settings() -> Result<Value, AppError> {
    let path = config_path();
    if !path.exists() {
        return Err(AppError::localized(
            "kimicode.config.missing",
            "Kimi Code 配置文件不存在",
            "Kimi Code configuration file is missing",
        ));
    }
    let doc = read_config(&path)?;
    let Some(alias) = doc.get("default_model").and_then(Item::as_str) else {
        return Ok(Value::Object(Default::default()));
    };
    let Some(provider_id) = provider_of_alias(&doc, alias) else {
        return Ok(Value::Object(Default::default()));
    };
    Ok(projection_to_settings(&doc, provider_id, Some(alias)))
}

/// Rebuild the stored settings JSON for one provider from a live document.
pub(crate) fn projection_to_settings(
    doc: &DocumentMut,
    provider_id: &str,
    default_alias: Option<&str>,
) -> Value {
    let Some(provider_table) = doc
        .get("providers")
        .and_then(Item::as_table_like)
        .and_then(|providers| providers.get(provider_id))
        .and_then(Item::as_table_like)
    else {
        return Value::Object(Default::default());
    };

    let mut settings = serde_json::Map::new();
    for (key, item) in provider_table.iter() {
        if let Some(value) = item.as_value().and_then(toml_value_to_json) {
            settings.insert(key.to_string(), value);
        } else if let Some(table) = item.as_table() {
            // Regular (non-inline) sub-tables such as [providers.x.custom_headers].
            let mut map = serde_json::Map::new();
            for (sub_key, sub_item) in table.iter() {
                if let Some(value) = sub_item.as_value().and_then(toml_value_to_json) {
                    map.insert(sub_key.to_string(), value);
                }
            }
            settings.insert(key.to_string(), Value::Object(map));
        }
    }

    let mut models = serde_json::Map::new();
    if let Some(all_models) = doc.get("models").and_then(Item::as_table_like) {
        for (alias, item) in all_models.iter() {
            let Some(table) = item.as_table_like() else {
                continue;
            };
            if table.get("provider").and_then(Item::as_str) != Some(provider_id) {
                continue;
            }
            let mut entry = serde_json::Map::new();
            for (key, value) in table.iter() {
                if key == "provider" {
                    continue;
                }
                if let Some(json) = value.as_value().and_then(toml_value_to_json) {
                    entry.insert(key.to_string(), json);
                }
            }
            models.insert(alias.to_string(), Value::Object(entry));
        }
    }
    settings.insert("models".to_string(), Value::Object(models));
    if let Some(alias) = default_alias {
        settings.insert(
            "default_model".to_string(),
            Value::String(alias.to_string()),
        );
    }
    Value::Object(settings)
}

fn toml_value_to_json(value: &toml_edit::Value) -> Option<Value> {
    match value {
        toml_edit::Value::String(s) => Some(Value::String(s.value().clone())),
        toml_edit::Value::Integer(i) => Some(Value::Number((*i.value()).into())),
        toml_edit::Value::Float(f) => serde_json::Number::from_f64(*f.value()).map(Value::Number),
        toml_edit::Value::Boolean(b) => Some(Value::Bool(*b.value())),
        toml_edit::Value::Array(array) => Some(Value::Array(
            array.iter().filter_map(toml_value_to_json).collect(),
        )),
        toml_edit::Value::InlineTable(table) => {
            let mut map = serde_json::Map::new();
            for (key, item) in table.iter() {
                if let Some(value) = toml_value_to_json(item) {
                    map.insert(key.to_string(), value);
                }
            }
            Some(Value::Object(map))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_provider() -> Value {
        json!({
            "type": "kimi",
            "base_url": "https://api.moonshot.ai/v1",
            "api_key": "sk-test",
            "models": {
                "kimi-for-coding": {
                    "model": "kimi-for-coding",
                    "max_context_size": 262144,
                    "capabilities": ["thinking", "tool_use"]
                }
            }
        })
    }

    #[test]
    fn validate_provider_accepts_sample_and_rejects_bad_input() {
        validate_provider("kimi-official", &sample_provider()).unwrap();
        for bad in [
            json!({"type": "unknown", "api_key": "k", "models": {"m": {"model": "m", "max_context_size": 1}}}),
            json!({"type": "kimi", "models": {"m": {"model": "m", "max_context_size": 1}}}),
            json!({"type": "kimi", "api_key": "k", "models": {}}),
            json!({"type": "kimi", "api_key": "k", "models": {"m": {"model": "m"}}}),
            json!({"type": "kimi", "api_key": "k", "base_url": "not a url", "models": {"m": {"model": "m", "max_context_size": 1}}}),
        ] {
            assert!(validate_provider("id", &bad).is_err(), "{bad}");
        }
        assert!(validate_provider("bad key!", &sample_provider()).is_err());
    }

    #[test]
    fn write_and_remove_preserves_unrelated_sections_and_quotes_special_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "# user comment\ndefault_permission_mode = \"manual\"\n\n[permission]\ndangerous_command_guard = true\n",
        )
        .unwrap();

        let mut doc = read_config(&path).unwrap();
        apply_projection(
            &mut doc,
            "managed:kimi-code",
            &json!({
                "type": "kimi",
                "base_url": "https://api.kimi.com/coding/v1",
                "api_key": "sk-x",
                "models": {
                    "kimi-code/k3": {"model": "k3", "max_context_size": 1048576},
                    "kimi-code/kimi-for-coding": {"model": "kimi-for-coding", "max_context_size": 262144}
                },
                "default_model": "kimi-code/kimi-for-coding"
            }),
        )
        .unwrap();
        write_config(&path, &doc).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("# user comment"));
        assert!(text.contains("[permission]"));
        assert!(text.contains("default_model = \"kimi-code/kimi-for-coding\""));
        assert!(
            text.contains("[providers.\"managed:kimi-code\"]"),
            "special chars must be quoted: {text}"
        );
        assert!(text.contains("[models.\"kimi-code/k3\"]"));

        // Round-trip the projection back into settings JSON.
        let doc = read_config(&path).unwrap();
        let settings =
            projection_to_settings(&doc, "managed:kimi-code", Some("kimi-code/kimi-for-coding"));
        assert_eq!(settings["type"], "kimi");
        assert_eq!(
            settings["models"]["kimi-code/k3"]["max_context_size"],
            1048576
        );
        assert_eq!(settings["default_model"], "kimi-code/kimi-for-coding");

        // Removing the provider must keep user-owned sections and drop the
        // dangling default_model.
        let mut doc = read_config(&path).unwrap();
        remove_projection(&mut doc, "managed:kimi-code");
        write_config(&path, &doc).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("providers"));
        assert!(!text.contains("default_model"));
        assert!(text.contains("[permission]"));
        assert!(text.contains("# user comment"));
    }

    #[test]
    fn stale_providers_are_removed_but_user_entries_survive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "[providers.\"user-owned\"]\ntype = \"openai\"\n\n[models.\"user-owned/m\"]\nprovider = \"user-owned\"\nmodel = \"m\"\nmax_context_size = 1\n",
        )
        .unwrap();
        let mut doc = read_config(&path).unwrap();
        apply_projection(&mut doc, "old", &sample_provider()).unwrap();
        write_config(&path, &doc).unwrap();

        // Switch: old is stale and must go, user-owned stays.
        let mut doc = read_config(&path).unwrap();
        remove_projection(&mut doc, "old");
        apply_projection(&mut doc, "new", &sample_provider()).unwrap();
        write_config(&path, &doc).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("[providers.old]"));
        assert!(text.contains("[providers.new]"));
        assert!(text.contains("[providers.\"user-owned\"]"));
        assert!(text.contains("[models.\"user-owned/m\"]"));
    }

    #[test]
    fn snippet_merge_refuses_provider_owned_sections() {
        let mut doc = DocumentMut::new();
        merge_toml_snippet(&mut doc, "[thinking]\nenabled = false\n").unwrap();
        assert_eq!(doc["thinking"]["enabled"].as_bool(), Some(false));
        assert!(merge_toml_snippet(&mut doc, "default_model = \"x\"").is_err());
        assert!(merge_toml_snippet(&mut doc, "[providers.x]\ntype=\"kimi\"").is_err());
    }
}
