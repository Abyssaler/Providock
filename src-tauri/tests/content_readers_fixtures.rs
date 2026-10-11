//! 用合成夹具驱动正文读取器（fixtures/content/）。
//! 预期值由人工按各源记录形状独立计算（见 fixtures/content/README.md），不由读取器反推。
//!
//! 每个用例覆盖：块数、role 序列、text 是否存在、truncated 标志、工具名与可折叠标志、
//! 分页两页取完（has_more 与字节游标），以及缺失/未知名目的诊断原因。

use providock_lib::adapters::content::{
    claude_code_content, codex_content, kimi_content, MessagePage,
};
use serde_json::Value;
use std::path::{Path, PathBuf};

type Reader = fn(&[u8], Option<u64>, usize) -> MessagePage;

fn reader_for(source: &str) -> Reader {
    match source {
        "claude_code" => claude_code_content::list_messages,
        "codex" => codex_content::list_messages,
        "kimi_code" => kimi_content::list_messages,
        other => panic!("unknown content source dir: {other}"),
    }
}

fn content_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/content")
}

fn single_input_file(case_dir: &Path) -> PathBuf {
    let input_dir = case_dir.join("input");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&input_dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", input_dir.display()))
        .filter_map(|e| {
            let p = e.unwrap().path();
            p.is_file().then_some(p)
        })
        .collect();
    files.sort();
    assert_eq!(files.len(), 1, "case {} needs exactly one input file", case_dir.display());
    files.pop().unwrap()
}

fn assert_page(case: &str, idx: usize, page: &MessagePage, spec: &Value, bytes: &[u8]) {
    let want_blocks = spec["blocks"].as_u64().unwrap() as usize;
    assert_eq!(page.blocks.len(), want_blocks, "{case} page{idx} block count");

    if let Some(roles) = spec.get("roles").and_then(|r| r.as_array()) {
        let got: Vec<String> = page.blocks.iter().map(|b| b.role.label()).collect();
        let want: Vec<String> = roles
            .iter()
            .map(|r| r.as_str().unwrap().to_string())
            .collect();
        assert_eq!(got, want, "{case} page{idx} roles");
    }

    let flags = |key: &str| -> Option<Vec<bool>> {
        spec.get(key)
            .and_then(|v| v.as_array())
            .map(|a| a.iter().map(|x| x.as_bool().unwrap()).collect())
    };
    if let Some(want) = flags("text_present") {
        let got: Vec<bool> = page.blocks.iter().map(|b| b.text.is_some()).collect();
        assert_eq!(got, want, "{case} page{idx} text_present");
    }
    if let Some(want) = flags("truncated") {
        let got: Vec<bool> = page.blocks.iter().map(|b| b.truncated).collect();
        assert_eq!(got, want, "{case} page{idx} truncated");
    }
    if let Some(want) = flags("collapsible") {
        let got: Vec<bool> = page.blocks.iter().map(|b| b.is_collapsible_tool).collect();
        assert_eq!(got, want, "{case} page{idx} collapsible");
    }
    if let Some(names) = spec.get("tool_names").and_then(|v| v.as_array()) {
        let got: Vec<Option<String>> = page.blocks.iter().map(|b| b.tool_name.clone()).collect();
        let want: Vec<Option<String>> = names
            .iter()
            .map(|n| n.as_str().map(str::to_string))
            .collect();
        assert_eq!(got, want, "{case} page{idx} tool_names");
    }

    let want_more = spec["has_more"].as_bool().unwrap();
    assert_eq!(page.has_more, want_more, "{case} page{idx} has_more");

    match spec.get("next_cursor_before_marker").and_then(|m| m.as_str()) {
        Some(marker) => {
            let pos = bytes
                .windows(marker.len())
                .position(|w| w == marker.as_bytes())
                .unwrap_or_else(|| panic!("{case}: marker not found: {marker:?}")) as u64;
            assert_eq!(page.next_cursor, Some(pos), "{case} page{idx} next_cursor");
        }
        None => assert_eq!(page.next_cursor, None, "{case} page{idx} next_cursor"),
    }

    for needle in spec["diagnostics_include"].as_array().cloned().unwrap_or_default() {
        let needle = needle.as_str().unwrap();
        assert!(
            page.diagnostics.iter().any(|d| d.contains(needle)),
            "{case} page{idx} diagnostics missing {needle:?}: {:?}",
            page.diagnostics
        );
    }

    // 安全：诊断里不得出现原始 JSON（`{`）或换行。
    for d in &page.diagnostics {
        assert!(
            !d.contains('{') && !d.contains('\n'),
            "{case} page{idx} diagnostic looks like raw JSON: {d:?}"
        );
    }

    // truncated=true 时文本长度必须等于上限（截断到常量，不超限）。
    for b in &page.blocks {
        if b.truncated {
            let len = b.text.as_ref().unwrap().chars().count();
            assert_eq!(
                len,
                providock_lib::adapters::content::MAX_TEXT_CHARS,
                "{case} page{idx} truncated block length"
            );
        }
    }
}

#[test]
fn content_readers_fixture_cases() {
    let root = content_root();
    let mut cases: Vec<(String, PathBuf)> = Vec::new();
    for source in std::fs::read_dir(&root).unwrap() {
        let source = source.unwrap().path();
        if !source.is_dir() {
            continue;
        }
        let source_name = source.file_name().unwrap().to_string_lossy().into_owned();
        for case in std::fs::read_dir(&source).unwrap() {
            let case = case.unwrap().path();
            if case.is_dir() {
                cases.push((source_name.clone(), case));
            }
        }
    }
    cases.sort();
    assert!(
        cases.len() >= 3,
        "expected at least one fixture per source, found {cases:?}"
    );

    for (source, case_dir) in cases {
        let case = case_dir.file_name().unwrap().to_string_lossy().into_owned();
        let label = format!("{source}/{case}");
        let reader = reader_for(&source);
        let bytes = std::fs::read(single_input_file(&case_dir)).unwrap();
        let expected: Value =
            serde_json::from_slice(&std::fs::read(case_dir.join("expected.json")).unwrap()).unwrap();
        let limit = expected["limit"].as_u64().unwrap() as usize;

        let mut cursor: Option<u64> = None;
        let pages = expected["pages"].as_array().unwrap();
        for (idx, spec) in pages.iter().enumerate() {
            let page = reader(&bytes, cursor, limit);
            assert_page(&label, idx, &page, spec, &bytes);
            cursor = page.next_cursor;
            if !page.has_more {
                assert_eq!(cursor, None, "{label} page{idx}: no more must clear cursor");
            }
        }
        assert_eq!(cursor, None, "{label}: pages must exhaust the file");
    }
}
