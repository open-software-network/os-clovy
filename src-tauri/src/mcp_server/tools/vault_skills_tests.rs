use super::{call, descriptors, ActivityAccess, McpData, ToolCtx};
use crate::agent_sessions::AgentRepository;
use crate::approval_broker::testing::TestBroker;
use crate::llm::cli::CliKind;
use serde_json::json;

async fn routine_fixture() -> (McpData, crate::mcp_server::turns::TurnBinding) {
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, false).await;
    let pool = broker.pool().clone();
    let now = "2026-10-28T00:00:00Z";
    sqlx::query::query("INSERT INTO routines (id, name, prompt, schedule, created_at, updated_at, metadata_json, tool_catalog_version) VALUES ('skills-routine', 'Daily', 'Summarise', 'every 1d', ?, ?, '{\"enabledToolsets\":[]}', 1)")
        .bind(now).bind(now).execute(&pool).await.unwrap();
    sqlx::query::query("INSERT INTO routine_runs (id, routine_id, agent_session_id, claim_token, trigger_kind, status, model, safety_mode, created_at, updated_at) VALUES ('skills-run', 'skills-routine', ?, 'claim', 'schedule', 'running', 'claude', 'sandboxed', ?, ?)")
        .bind(&binding.session_id).bind(now).bind(now).execute(&pool).await.unwrap();
    (
        McpData {
            notes: pool,
            profile: "default".into(),
            memory_enabled: true,
            activity: ActivityAccess::Off,
            now: chrono::Utc::now(),
        },
        binding,
    )
}

async fn routine_skills_enabled(data: &McpData, enabled: bool) {
    sqlx::query::query("UPDATE routines SET metadata_json = ? WHERE id = 'skills-routine'")
        .bind(json!({"enabledToolsets": if enabled { vec!["skills"] } else { vec![] }}).to_string())
        .execute(&data.notes)
        .await
        .unwrap();
}

#[tokio::test]
async fn routine_vault_discovery_does_not_require_the_skill_toolset() {
    let (data, binding) = routine_fixture().await;
    let ctx = ToolCtx {
        binding: Some(&binding),
        ..ToolCtx::unbound(&data)
    };
    assert!(descriptors(&ctx)
        .await
        .iter()
        .any(|tool| tool["name"] == "get_obsidian_vault"));
    // Passing policy reaches the native-app boundary; no vault discovery
    // or real user directories are accessed by this synthetic fixture.
    assert_eq!(
        call(&ctx, "get_obsidian_vault", &json!({}))
            .await
            .unwrap_err()
            .code,
        "mcp_app_unavailable"
    );
}

#[tokio::test]
async fn routine_skill_catalog_obeys_saved_toolsets() {
    let (data, binding) = routine_fixture().await;
    let ctx = ToolCtx {
        binding: Some(&binding),
        ..ToolCtx::unbound(&data)
    };
    let names = |catalog: Vec<serde_json::Value>| {
        catalog
            .into_iter()
            .filter_map(|v| v["name"].as_str().map(str::to_owned))
            .filter(|v| matches!(v.as_str(), "list_skills" | "load_skill"))
            .collect::<Vec<_>>()
    };
    assert!(names(descriptors(&ctx).await).is_empty());
    routine_skills_enabled(&data, true).await;
    assert_eq!(
        names(descriptors(&ctx).await),
        ["list_skills", "load_skill"]
    );
    routine_skills_enabled(&data, false).await;
    assert!(names(descriptors(&ctx).await).is_empty());
}

#[tokio::test]
async fn routine_skill_dispatch_rechecks_policy_before_arguments_and_files() {
    let (data, binding) = routine_fixture().await;
    let ctx = ToolCtx {
        binding: Some(&binding),
        ..ToolCtx::unbound(&data)
    };
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("example")).unwrap();
    std::fs::write(
        root.path().join("example/SKILL.md"),
        "Synthetic instructions",
    )
    .unwrap();
    let roots = vec![root.path().to_path_buf()];
    for enabled_before_revoke in [false, true] {
        if enabled_before_revoke {
            routine_skills_enabled(&data, true).await;
            let loaded = super::vault_skills::call_with_roots(
                &ctx,
                "load_skill",
                &json!({"name":"example"}),
                &roots,
            )
            .await
            .unwrap();
            assert_eq!(loaded["content"], "Synthetic instructions");
            routine_skills_enabled(&data, false).await;
        }
        for name in ["list_skills", "load_skill"] {
            assert_eq!(
                call(&ctx, name, &serde_json::Value::Null)
                    .await
                    .unwrap_err()
                    .code,
                "routine_tool_not_enabled"
            );
            assert_eq!(
                super::vault_skills::call_with_roots(&ctx, name, &serde_json::Value::Null, &roots)
                    .await
                    .unwrap_err()
                    .code,
                "routine_tool_not_enabled"
            );
        }
    }
}

#[tokio::test]
async fn skills_follow_global_settings_and_preserve_root_precedence() {
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let managed = tempfile::tempdir().unwrap();
    let global = tempfile::tempdir().unwrap();
    for (root, name, content) in [
        (managed.path(), "enabled", "Managed instructions"),
        (global.path(), "enabled", "Global duplicate"),
        (global.path(), "disabled", "Disabled instructions"),
        (global.path(), "new-skill", "Default enabled instructions"),
    ] {
        std::fs::create_dir_all(root.join(name)).unwrap();
        std::fs::write(root.join(name).join("SKILL.md"), content).unwrap();
    }
    let roots = vec![managed.path().to_path_buf(), global.path().to_path_buf()];
    let repo = AgentRepository::new(broker.pool().clone());
    repo.set_skill_enabled("disabled", false, false)
        .await
        .unwrap();
    let data = McpData {
        notes: broker.pool().clone(),
        profile: "default".into(),
        memory_enabled: true,
        activity: ActivityAccess::Off,
        now: chrono::Utc::now(),
    };
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: None,
    };
    let listed = super::vault_skills::call_with_roots(&ctx, "list_skills", &json!({}), &roots)
        .await
        .unwrap();
    let skills = listed["skills"].as_array().unwrap();
    assert_eq!(skills.len(), 2);
    assert!(skills.iter().all(|skill| skill["name"] != "disabled"));
    let loaded = super::vault_skills::call_with_roots(
        &ctx,
        "load_skill",
        &json!({"name":"enabled"}),
        &roots,
    )
    .await
    .unwrap();
    assert_eq!(loaded["content"], "Managed instructions");
    assert_eq!(
        loaded["path"],
        managed
            .path()
            .join("enabled/SKILL.md")
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
    );
    assert_eq!(
        super::vault_skills::call_with_roots(
            &ctx,
            "load_skill",
            &json!({"name":"disabled"}),
            &roots
        )
        .await
        .unwrap_err()
        .code,
        "agent_skill_disabled"
    );
    repo.set_skill_enabled("disabled", true, false)
        .await
        .unwrap();
    assert_eq!(
        super::vault_skills::call_with_roots(
            &ctx,
            "load_skill",
            &json!({"name":"disabled"}),
            &roots
        )
        .await
        .unwrap()["content"],
        "Disabled instructions"
    );
    assert!(broker.requests().is_empty());
}

#[tokio::test]
async fn skills_preserve_aliases_validation_and_native_snapshot_policy() {
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Codex, true).await;
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("june-obsidian")).unwrap();
    std::fs::write(
        root.path().join("june-obsidian/SKILL.md"),
        "Legacy instructions",
    )
    .unwrap();
    let roots = vec![root.path().to_path_buf()];
    let data = McpData {
        notes: broker.pool().clone(),
        profile: "default".into(),
        memory_enabled: true,
        activity: ActivityAccess::Off,
        now: chrono::Utc::now(),
    };
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: None,
    };
    assert_eq!(
        super::vault_skills::call_with_roots(
            &ctx,
            "load_skill",
            &json!({"name":"clovy-obsidian"}),
            &roots
        )
        .await
        .unwrap()["content"],
        "Legacy instructions"
    );
    std::fs::create_dir_all(root.path().join("clovy-obsidian")).unwrap();
    std::fs::write(
        root.path().join("clovy-obsidian/SKILL.md"),
        "Canonical instructions",
    )
    .unwrap();
    assert_eq!(
        super::vault_skills::call_with_roots(&ctx, "list_skills", &json!({}), &roots)
            .await
            .unwrap()["skills"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        super::vault_skills::call_with_roots(
            &ctx,
            "load_skill",
            &json!({"name":"june-obsidian"}),
            &roots
        )
        .await
        .unwrap()["content"],
        "Canonical instructions"
    );
    for name in ["../escape", "a/b", "a\\b", ".", ".."] {
        assert_eq!(
            super::vault_skills::call_with_roots(&ctx, "load_skill", &json!({"name":name}), &roots)
                .await
                .unwrap_err()
                .code,
            "agent_skill_invalid"
        );
    }
    let snapshot = std::collections::HashMap::new();
    assert!(
        super::vault_skills::list_skills_from_roots(&roots, &snapshot, false)
            .await
            .unwrap()["skills"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        super::vault_skills::load_skill_from_roots(&roots, &snapshot, "clovy-obsidian")
            .await
            .unwrap_err()
            .code,
        "agent_skill_disabled"
    );
    for name in ["get_obsidian_vault", "list_skills", "load_skill"] {
        assert_eq!(
            call(&ToolCtx::unbound(&data), name, &json!({}))
                .await
                .unwrap_err()
                .code,
            "turn_required"
        );
        assert!(!descriptors(&ToolCtx::unbound(&data))
            .await
            .iter()
            .any(|tool| tool["name"] == name));
        assert!(descriptors(&ctx)
            .await
            .iter()
            .any(|tool| tool["name"] == name && tool["annotations"]["readOnlyHint"] == true));
    }
}

#[test]
fn vault_result_preserves_discovery_contract_for_test_vault() {
    let vault = tempfile::tempdir().unwrap();
    std::fs::create_dir(vault.path().join(".obsidian")).unwrap();
    let path = vault.path().canonicalize().unwrap();
    let result = super::vault_skills::vault_result(Ok(crate::obsidian::ObsidianDiscovery {
        connected: true,
        available: true,
        vault: Some(crate::obsidian::ObsidianDiscoveryVault {
            name: "Test vault".into(),
            path: Some(path.to_str().unwrap().into()),
        }),
    }))
    .unwrap();
    assert_eq!(
        result,
        json!({"connected":true,"available":true,"vault":{"name":"Test vault","path":path}})
    );
    let unavailable = super::vault_skills::vault_result(Ok(crate::obsidian::ObsidianDiscovery {
        connected: true,
        available: false,
        vault: Some(crate::obsidian::ObsidianDiscoveryVault {
            name: "Test vault".into(),
            path: None,
        }),
    }))
    .unwrap();
    assert!(unavailable["vault"].get("path").is_none());
}

#[tokio::test]
async fn skill_admin_uses_ui_catalog_disabled_state_and_managed_boundary() {
    use crate::approval_broker::testing::Scripted;
    let broker = TestBroker::new([Scripted::Approve, Scripted::Approve, Scripted::Approve]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let managed = tempfile::tempdir().unwrap();
    let global = tempfile::tempdir().unwrap();
    for (root, id, content) in [
        (managed.path(), "managed", "Original"),
        (global.path(), "global-only", "Global"),
        (managed.path(), "foo-backup", "Legitimate"),
        (managed.path(), ".creative-x.backup-1", "Hidden"),
    ] {
        std::fs::create_dir_all(root.join(id)).unwrap();
        std::fs::write(root.join(id).join("SKILL.md"), content).unwrap();
    }
    let roots = vec![managed.path().to_path_buf(), global.path().to_path_buf()];
    let data = McpData {
        notes: broker.pool().clone(),
        profile: "default".into(),
        memory_enabled: true,
        activity: ActivityAccess::Off,
        now: chrono::Utc::now(),
    };
    let ctx = ToolCtx {
        binding: Some(&binding),
        broker: Some(broker.broker()),
        ..ToolCtx::unbound(&data)
    };
    let args = json!({"name":"managed"});
    super::vault_skills::call_with_roots(&ctx, "disable_skill", &args, &roots)
        .await
        .unwrap();
    let settings =
        super::vault_skills::call_with_roots(&ctx, "list_skill_settings", &json!({}), &roots)
            .await
            .unwrap();
    let ui_catalog = crate::agent_sessions::skills::catalog_from_roots(
        vec![(roots[0].clone(), true), (roots[1].clone(), false)],
        &std::collections::HashMap::from([("managed".to_string(), false)]),
    )
    .await
    .unwrap();
    assert_eq!(settings["skills"], json!(ui_catalog));
    assert!(
        !super::vault_skills::call_with_roots(&ctx, "list_skills", &json!({}), &roots)
            .await
            .unwrap()["skills"]
            .as_array()
            .unwrap()
            .iter()
            .any(|skill| skill["name"] == "managed")
    );
    assert_eq!(
        super::vault_skills::call_with_roots(&ctx, "read_skill", &args, &roots)
            .await
            .unwrap()["content"],
        "Original"
    );
    assert_eq!(
        super::vault_skills::call_with_roots(&ctx, "load_skill", &args, &roots)
            .await
            .unwrap_err()
            .code,
        "agent_skill_disabled"
    );
    let edited = super::vault_skills::call_with_roots(
        &ctx,
        "update_skill",
        &json!({"name":"managed","content":"Revised"}),
        &roots,
    )
    .await
    .unwrap();
    assert_eq!(edited["enabled"], false);
    super::vault_skills::call_with_roots(&ctx, "enable_skill", &args, &roots)
        .await
        .unwrap();
    assert!(
        super::vault_skills::call_with_roots(&ctx, "load_skill", &args, &roots)
            .await
            .unwrap()["content"]
            .as_str()
            .unwrap()
            .ends_with("Revised")
    );
    for name in ["update_skill", "enable_skill", "disable_skill"] {
        assert_eq!(
            super::vault_skills::call_with_roots(
                &ctx,
                name,
                &json!({"name":"global-only","content":"Denied"}),
                &roots
            )
            .await
            .unwrap_err()
            .code,
            "agent_skill_read_only"
        );
    }
    assert_eq!(broker.requests().len(), 3);
    assert!(managed
        .path()
        .join(".creative-x.backup-1/SKILL.md")
        .exists());
    assert_eq!(
        std::fs::read_to_string(global.path().join("global-only/SKILL.md")).unwrap(),
        "Global"
    );
}

#[tokio::test]
async fn conflicting_alias_settings_keep_canonical_disabled_and_invalid_content_in_admin_only() {
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let managed = tempfile::tempdir().unwrap();
    let global = tempfile::tempdir().unwrap();
    for (root, id) in [
        (managed.path(), "clovy-obsidian"),
        (managed.path(), "valid"),
        (global.path(), "oversized"),
        (global.path(), "invalid-utf8"),
    ] {
        std::fs::create_dir(root.join(id)).unwrap();
    }
    std::fs::write(managed.path().join("clovy-obsidian/SKILL.md"), "Canonical").unwrap();
    std::fs::write(managed.path().join("valid/SKILL.md"), "Valid").unwrap();
    std::fs::write(
        global.path().join("oversized/SKILL.md"),
        vec![b'x'; 512 * 1024 + 1],
    )
    .unwrap();
    std::fs::write(global.path().join("invalid-utf8/SKILL.md"), [0xff]).unwrap();
    for (id, enabled) in [
        ("clovy-obsidian", false),
        ("june-obsidian", true),
        ("oversized", false),
    ] {
        sqlx::query::query("INSERT INTO agent_skill_settings(skill_id,enabled,managed,updated_at) VALUES (?,?,1,'now')").bind(id).bind(enabled).execute(broker.pool()).await.unwrap();
    }
    let data = McpData {
        notes: broker.pool().clone(),
        profile: "default".into(),
        memory_enabled: true,
        activity: ActivityAccess::Off,
        now: chrono::Utc::now(),
    };
    let ctx = ToolCtx {
        binding: Some(&binding),
        ..ToolCtx::unbound(&data)
    };
    let roots = vec![managed.path().into(), global.path().into()];
    let active = super::vault_skills::call_with_roots(&ctx, "list_skills", &json!({}), &roots)
        .await
        .unwrap();
    assert_eq!(
        active["skills"]
            .as_array()
            .unwrap()
            .iter()
            .map(|skill| skill["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["valid"]
    );
    let admin =
        super::vault_skills::call_with_roots(&ctx, "list_skill_settings", &json!({}), &roots)
            .await
            .unwrap();
    assert_eq!(admin["skills"].as_array().unwrap().len(), 4);
    assert_eq!(
        admin["skills"]
            .as_array()
            .unwrap()
            .iter()
            .find(|skill| skill["id"] == "clovy-obsidian")
            .unwrap()["configuredEnabled"],
        false
    );
    assert_eq!(
        super::vault_skills::call_with_roots(
            &ctx,
            "load_skill",
            &json!({"name":"june-obsidian"}),
            &roots
        )
        .await
        .unwrap_err()
        .code,
        "agent_skill_disabled"
    );
    assert_eq!(
        super::vault_skills::call_with_roots(
            &ctx,
            "read_skill",
            &json!({"name":"oversized"}),
            &roots
        )
        .await
        .unwrap_err()
        .code,
        "agent_skill_too_large"
    );
    assert_eq!(
        super::vault_skills::call_with_roots(
            &ctx,
            "load_skill",
            &json!({"name":"invalid-utf8"}),
            &roots
        )
        .await
        .unwrap_err()
        .code,
        "agent_skill_read_failed"
    );
    let legacy: bool = sqlx::query_scalar::query_scalar(
        "SELECT enabled FROM agent_skill_settings WHERE skill_id='june-obsidian'",
    )
    .fetch_one(broker.pool())
    .await
    .unwrap();
    assert!(legacy);
}
