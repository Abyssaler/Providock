use providock_lib::{AppType, McpServer, McpService, Provider, ProviderService};
use serde_json::json;
use std::fs;

#[path = "support.rs"]
mod support;
use support::{create_test_state, ensure_test_home, reset_test_fs, test_mutex};

fn kimi_config_path() -> std::path::PathBuf {
    ensure_test_home().join(".kimi-code").join("config.toml")
}

fn kimi_provider(id: &str, model: &str) -> Provider {
    Provider::with_id(
        id.to_string(),
        format!("Kimi {id}"),
        json!({
            "type": "kimi",
            "base_url": format!("https://{id}.example.com/v1"),
            "api_key": format!("sk-{id}"),
            "models": {
                model: {
                    "model": model,
                    "max_context_size": 262144
                }
            }
        }),
        None,
    )
}

#[test]
fn kimicode_switch_projects_provider_and_preserves_user_sections() {
    let _guard = test_mutex().lock().unwrap();
    reset_test_fs();
    let state = create_test_state().unwrap();

    // Seed a config.toml with unrelated user content that must survive.
    let path = kimi_config_path();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        "# user note\ndefault_permission_mode = \"manual\"\n\n[permission]\ndangerous_command_guard = true\n",
    )
    .unwrap();

    // First provider becomes current on add (no current exists yet).
    ProviderService::add(
        &state,
        AppType::KimiCode,
        kimi_provider("alpha", "kimi-for-coding"),
        false,
    )
    .expect("add first provider");
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("[providers.alpha]"), "{text}");
    assert!(
        text.contains("default_model = \"kimi-for-coding\""),
        "{text}"
    );
    assert!(text.contains("# user note"), "{text}");
    assert!(text.contains("[permission]"), "{text}");
    assert_eq!(
        ProviderService::current(&state, AppType::KimiCode).unwrap(),
        "alpha"
    );

    // Add a second provider and switch: alpha's projection must be removed,
    // beta's written, and user content still preserved.
    ProviderService::add(
        &state,
        AppType::KimiCode,
        kimi_provider("beta", "k3"),
        false,
    )
    .expect("add second provider");
    ProviderService::switch(&state, AppType::KimiCode, "beta").expect("switch to beta");
    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains("[providers.alpha]"), "{text}");
    assert!(text.contains("[providers.beta]"), "{text}");
    assert!(text.contains("default_model = \"k3\""), "{text}");
    assert!(text.contains("[permission]"), "{text}");

    // Deleting the non-current provider is a no-op on live (it has no
    // projection); deleting the current one is refused.
    ProviderService::delete(&state, AppType::KimiCode, "alpha").expect("delete non-current");
    assert!(
        ProviderService::delete(&state, AppType::KimiCode, "beta").is_err(),
        "current provider deletion must be refused"
    );
}

#[test]
fn kimicode_mcp_sync_and_toggle() {
    let _guard = test_mutex().lock().unwrap();
    reset_test_fs();
    let state = create_test_state().unwrap();
    let mcp_path = ensure_test_home().join(".kimi-code").join("mcp.json");

    // Seed an unmanaged server that must survive syncs.
    fs::create_dir_all(mcp_path.parent().unwrap()).unwrap();
    fs::write(
        &mcp_path,
        json!({"mcpServers": {"keep": {"url": "https://example.com/mcp"}}}).to_string(),
    )
    .unwrap();

    let server = McpServer {
        id: "kimi-sse".into(),
        name: "Kimi SSE".into(),
        server: json!({"type": "sse", "url": "https://example.com/sse", "headers": {"X": "y"}}),
        apps: providock_lib::McpApps {
            kimicode: true,
            ..Default::default()
        },
        description: None,
        homepage: None,
        docs: None,
        tags: vec![],
    };
    McpService::upsert_server(&state, server).expect("upsert kimicode mcp");

    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&mcp_path).unwrap()).unwrap();
    assert_eq!(written["mcpServers"]["kimi-sse"]["transport"], "sse");
    assert_eq!(written["mcpServers"]["kimi-sse"]["enabled"], true);
    assert_eq!(
        written["mcpServers"]["keep"]["url"],
        "https://example.com/mcp"
    );

    // Toggling off removes the projection but keeps unmanaged entries.
    McpService::toggle_app(&state, "kimi-sse", AppType::KimiCode, false).expect("toggle off");
    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&mcp_path).unwrap()).unwrap();
    assert!(written["mcpServers"].get("kimi-sse").is_none());
    assert!(written["mcpServers"].get("keep").is_some());
    assert!(
        !state.db.get_all_mcp_servers().unwrap()["kimi-sse"]
            .apps
            .kimicode
    );
}

#[test]
fn kimicode_prompt_enable_writes_agents_md() {
    let _guard = test_mutex().lock().unwrap();
    reset_test_fs();
    let state = create_test_state().unwrap();
    let path = ensure_test_home().join(".kimi-code").join("AGENTS.md");

    let prompt: providock_lib::Prompt = serde_json::from_value(json!({
        "id": "p1", "name": "P1", "content": "# kimi rules", "enabled": false
    }))
    .unwrap();
    providock_lib::PromptService::upsert_prompt(&state, AppType::KimiCode, "p1", prompt.clone())
        .expect("save disabled prompt");
    assert!(
        !path.exists(),
        "disabled prompt must not write the live file"
    );

    providock_lib::PromptService::enable_prompt(&state, AppType::KimiCode, "p1")
        .expect("enable prompt");
    assert_eq!(fs::read_to_string(&path).unwrap(), "# kimi rules");
}
