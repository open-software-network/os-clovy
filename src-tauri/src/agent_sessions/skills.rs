//! Managed bundled skills and the local MCP subprocess sandbox.
pub(crate) use super::skill_catalog::catalog_from_roots;
use super::skill_catalog::skill_read_error;
use super::skill_identity::reconcile_managed_obsidian_skill;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
pub fn seed_bundled_skills(app: &AppHandle) {
    let Some(managed_root) = crate::app_paths::app_data_dir(app)
        .ok()
        .map(|path| path.join("agents").join("skills"))
    else {
        return;
    };
    if !super::skill_identity::managed_root_is_writable(&managed_root, &skill_roots(app)) {
        tracing::warn!(
            "managed skill storage aliases read-only global skills; bundled seeding skipped"
        );
        return;
    }
    let resource_root = app
        .path()
        .resource_dir()
        .ok()
        .map(|dir| dir.join("native").join("agent-skills"));
    let debug_root = std::env::var("CARGO_MANIFEST_DIR").ok().map(|manifest| {
        PathBuf::from(manifest)
            .join("resources")
            .join("agent-skills")
    });
    let source_root = resource_root.filter(|path| path.is_dir()).or(debug_root);
    let Some(source_root) = source_root else {
        tracing::warn!("could not resolve bundled agent-skills resource directory");
        return;
    };
    if let Err(error) = reconcile_managed_obsidian_skill(&managed_root) {
        tracing::warn!(?error, "could not promote the legacy Obsidian skill");
    }
    let Ok(entries) = std::fs::read_dir(&source_root) else {
        return;
    };
    for entry in entries.flatten() {
        let skill_file = entry.path().join("SKILL.md");
        if !skill_file.is_file() {
            continue;
        }
        let skill_id = entry.file_name().to_string_lossy().into_owned();
        if !super::skill_identity::valid_skill_id(&skill_id) {
            continue;
        }
        let dest_dir = managed_root.join(&skill_id);
        let dest_file = dest_dir.join("SKILL.md");
        if dest_file.is_file() {
            continue;
        }
        if let Err(error) = std::fs::create_dir_all(&dest_dir) {
            tracing::warn!(?error, skill = %skill_id, "could not create managed skill directory");
            continue;
        }
        let Ok(canonical_root) = managed_root.canonicalize() else {
            continue;
        };
        let Ok(canonical_dir) = dest_dir.canonicalize() else {
            continue;
        };
        if !canonical_dir.starts_with(&canonical_root) {
            continue;
        }
        let temp = canonical_dir.join(format!(".SKILL.{}.tmp", uuid::Uuid::new_v4()));
        if let Err(error) = std::fs::copy(&skill_file, &temp) {
            tracing::warn!(?error, skill = %skill_id, "could not copy bundled skill");
            continue;
        }
        if let Err(error) = std::fs::rename(&temp, canonical_dir.join("SKILL.md")) {
            tracing::warn!(?error, skill = %skill_id, "could not finalize bundled skill");
            let _ = std::fs::remove_file(&temp);
        }
    }
    if let Err(error) = reconcile_managed_obsidian_skill(&managed_root) {
        tracing::warn!(?error, "could not publish the legacy Obsidian skill alias");
    }
}

/// Shared UI, MCP and run-loader roots. The first root is managed and editable.
pub(crate) fn skill_roots(app: &AppHandle) -> Vec<(PathBuf, bool)> {
    let mut roots = crate::app_paths::app_data_dir(app)
        .ok()
        .map(|path| (path.join("agents").join("skills"), true))
        .into_iter()
        .collect::<Vec<_>>();
    if let Some(home) = std::env::var_os("HOME") {
        roots.push((PathBuf::from(home).join(".agents").join("skills"), false));
    }
    roots
}

pub(crate) async fn read_from_roots(
    roots: &[(PathBuf, bool)],
    id: &str,
) -> Result<serde_json::Value, crate::domain::types::AppError> {
    use super::skill_identity::{
        canonical_skill_id, discover_skills, read_skill_content, valid_skill_id,
    };
    use crate::domain::types::AppError;
    if !valid_skill_id(id) {
        return Err(AppError::new("agent_skill_invalid", "Skill id is invalid."));
    }
    let skill = discover_skills(roots)
        .into_iter()
        .find(|skill| skill.id == canonical_skill_id(id))
        .ok_or_else(|| AppError::new("agent_skill_not_found", "Skill was not found."))?;
    let content = read_skill_content(&skill.path).map_err(skill_read_error)?;
    Ok(
        serde_json::json!({"name":skill.id,"content":content,"path":skill.path,"readOnly":!skill.managed}),
    )
}

pub(crate) async fn update_from_roots(
    roots: &[(PathBuf, bool)],
    id: &str,
    content: &str,
) -> Result<(), crate::domain::types::AppError> {
    use super::skill_identity::{
        canonical_skill_id, discover_skills, update_managed_skill, valid_skill_id, MAX_SKILL_BYTES,
    };
    use crate::domain::types::AppError;
    if !valid_skill_id(id) {
        return Err(AppError::new("agent_skill_invalid", "Skill id is invalid."));
    }
    if content.len() > MAX_SKILL_BYTES {
        return Err(AppError::new(
            "agent_skill_too_large",
            "Skill content exceeds 512 KiB.",
        ));
    }
    let skill = discover_skills(roots)
        .into_iter()
        .find(|skill| skill.id == canonical_skill_id(id))
        .ok_or_else(|| AppError::new("agent_skill_not_found", "Skill was not found."))?;
    if !skill.managed {
        return Err(AppError::new(
            "agent_skill_read_only",
            "User-global skills are read-only in Clovy.",
        ));
    }
    let root = roots
        .iter()
        .find(|(root, managed)| {
            *managed
                && super::skill_identity::read_skill_ids(id)
                    .iter()
                    .any(|read_id| {
                        super::skill_identity::contained_skill_file(root, read_id)
                            .is_some_and(|path| path == skill.path)
                    })
        })
        .ok_or_else(|| {
            AppError::new(
                "agent_skill_read_only",
                "Managed skill storage is unavailable.",
            )
        })?;
    if !super::skill_identity::managed_root_is_writable(&root.0, roots) {
        return Err(AppError::new(
            "agent_skill_read_only",
            "Managed skill storage overlaps read-only global skills.",
        ));
    }
    update_managed_skill(&root.0, id, content)
        .map_err(|error| AppError::new("agent_skill_write_failed", error.to_string()))
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn sandbox_profile(workspace: &std::path::Path) -> String {
    let escaped = workspace
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!("(version 1) (allow default) (deny file-write*) (allow file-write* (subpath \"{escaped}\")) (allow file-write* (subpath \"/private/tmp\"))")
}

#[cfg(test)]
mod tests {
    use super::sandbox_profile;
    #[test]
    fn escape_regression_sandbox_profile_preserves_workspace_paths() {
        for path in [
            "/tmp/plain",
            "/tmp/quoted\"name",
            "/tmp/slash\\\"quote",
            "/tmp/trailing\\",
        ] {
            let profile = sandbox_profile(std::path::Path::new(path));
            let quoted = serde_json::to_string(path).unwrap();
            assert!(
                profile.contains(&format!("(subpath {quoted})")),
                "{profile}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn escape_regression_sandbox_exec_accepts_quoted_and_trailing_backslash_paths() {
        let directory = tempfile::tempdir().unwrap();
        for name in ["plain", "quoted\"name", "slash\\\"quote", "trailing\\"] {
            let workspace = directory.path().join(name);
            std::fs::create_dir(&workspace).unwrap();
            let output = std::process::Command::new("/usr/bin/sandbox-exec")
                .args(["-p", &sandbox_profile(&workspace), "/usr/bin/true"])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn sandbox_profile_only_grants_workspace_and_tmp_writes() {
        let profile = sandbox_profile(std::path::Path::new("/Users/example/Clovy Workspace"));
        assert!(profile.contains("(deny file-write*)"));
        assert!(profile.contains("/Users/example/Clovy Workspace"));
        assert!(!profile.contains("(allow file-write*)"));
    }
}

/// A bound skill update has one admission order with Stop. File replacement is
/// synchronous and bounded while the database writer reserves the caller run.
pub(crate) async fn update_for_turn(
    pool: &sqlx_sqlite::SqlitePool,
    roots: &[(PathBuf, bool)],
    id: &str,
    content: &str,
    turn: (
        &str,
        &str,
        &str,
        &crate::domain::turn_authority::TurnAuthority,
    ),
) -> Result<(), crate::domain::types::AppError> {
    let (profile, session, run, authority) = turn;
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    crate::db::repositories::guard_bound_turn(&mut transaction, profile, session, run, authority)
        .await?;
    update_from_roots(roots, id, content).await?;
    transaction.commit().await?;
    Ok(())
}

pub(crate) async fn set_enabled_from_roots(
    pool: &sqlx_sqlite::SqlitePool,
    roots: &[(PathBuf, bool)],
    id: &str,
    enabled: bool,
    turn: Option<(
        &str,
        &str,
        &str,
        &crate::domain::turn_authority::TurnAuthority,
    )>,
) -> Result<(), crate::domain::types::AppError> {
    use super::skill_identity::{canonical_skill_id, discover_skills, valid_skill_id};
    use crate::domain::types::AppError;
    if !valid_skill_id(id) {
        return Err(AppError::new("agent_skill_invalid", "Skill id is invalid."));
    }
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    if let Some((profile, session, run, authority)) = turn {
        crate::db::repositories::guard_bound_turn(
            &mut transaction,
            profile,
            session,
            run,
            authority,
        )
        .await?;
    }
    let id = canonical_skill_id(id);
    let skill = discover_skills(roots)
        .into_iter()
        .find(|skill| skill.id == id)
        .ok_or_else(|| AppError::new("agent_skill_not_found", "Skill was not found."))?;
    if !skill.managed {
        return Err(AppError::new(
            "agent_skill_read_only",
            "User-global skills are read-only in Clovy.",
        ));
    }
    sqlx::query::query("INSERT INTO agent_skill_settings(skill_id, enabled, managed, updated_at) VALUES (?, ?, 1, ?) ON CONFLICT(skill_id) DO UPDATE SET enabled = excluded.enabled, managed = excluded.managed, updated_at = excluded.updated_at")
        .bind(id).bind(enabled).bind(crate::db::repositories::timestamp()).execute(&mut *transaction).await?;
    transaction.commit().await?;
    Ok(())
}

#[cfg(test)]
mod bound_admin_tests {
    use super::*;
    use crate::{approval_broker::testing::TestBroker, llm::cli::CliKind};
    #[tokio::test]
    async fn stopped_turn_cannot_replace_or_enable_managed_skill() {
        let broker = TestBroker::new([]).await;
        let binding = broker.turn(CliKind::Claude, true).await;
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("example")).unwrap();
        let file = directory.path().join("example/SKILL.md");
        std::fs::write(&file, "Original").unwrap();
        let roots = vec![(directory.path().to_path_buf(), true)];
        sqlx::query::query("UPDATE agent_runs SET status = 'cancelled' WHERE id = ?")
            .bind(&binding.run_id)
            .execute(broker.pool())
            .await
            .unwrap();
        let turn = (
            "default",
            binding.session_id.as_str(),
            binding.run_id.as_str(),
            &binding.authority,
        );
        assert_eq!(
            update_for_turn(broker.pool(), &roots, "example", "Denied", turn)
                .await
                .unwrap_err()
                .code,
            "turn_expired"
        );
        assert_eq!(
            set_enabled_from_roots(broker.pool(), &roots, "example", false, Some(turn))
                .await
                .unwrap_err()
                .code,
            "turn_expired"
        );
        assert_eq!(std::fs::read_to_string(file).unwrap(), "Original");
        let count: i64 = sqlx::query_scalar::query_scalar(
            "SELECT COUNT(*) FROM agent_skill_settings WHERE skill_id='example'",
        )
        .fetch_one(broker.pool())
        .await
        .unwrap();
        assert_eq!(count, 0);
    }
}

#[cfg(test)]
mod catalog_content_regressions {
    use super::super::skill_catalog::skill_description;
    use super::super::skill_identity::MAX_SKILL_BYTES;
    use super::*;
    #[test]
    fn description_keeps_the_first_non_frontmatter_line() {
        for (input, expected) in [
            ("One line", "One line"),
            ("First line\nSecond line", "First line"),
            ("# Heading\nBody", "Body"),
            (
                "---\nname: example\ndescription: 'Summary'\n---\nBody",
                "Summary",
            ),
            ("---\nname: example\n---\n# Heading\nBody", "Body"),
        ] {
            assert_eq!(skill_description(input).as_deref(), Some(expected));
        }
    }
    #[tokio::test]
    async fn invalid_global_content_does_not_block_valid_admin_or_active_catalog() {
        let managed = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        for (root, id) in [
            (managed.path(), "valid"),
            (managed.path(), "repair"),
            (global.path(), "oversized"),
            (global.path(), "invalid-utf8"),
        ] {
            std::fs::create_dir(root.join(id)).unwrap();
        }
        std::fs::write(managed.path().join("valid/SKILL.md"), "Valid instructions").unwrap();
        std::fs::write(
            managed.path().join("repair/SKILL.md"),
            vec![b'x'; MAX_SKILL_BYTES + 1],
        )
        .unwrap();
        std::fs::write(
            global.path().join("oversized/SKILL.md"),
            vec![b'x'; MAX_SKILL_BYTES + 1],
        )
        .unwrap();
        std::fs::write(global.path().join("invalid-utf8/SKILL.md"), [0xff, 0xfe]).unwrap();
        let roots = vec![(managed.path().into(), true), (global.path().into(), false)];
        let overrides = std::collections::HashMap::from([("oversized".into(), false)]);
        let catalog = catalog_from_roots(roots.clone(), &overrides).await.unwrap();
        assert_eq!(catalog.len(), 4);
        assert_eq!(
            catalog.iter().find(|entry| entry["id"] == "valid").unwrap()["description"],
            "Valid instructions"
        );
        assert_eq!(
            catalog
                .iter()
                .filter(|entry| entry["enabled"] == true)
                .map(|entry| entry["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["valid"]
        );
        let invalid = catalog
            .iter()
            .find(|entry| entry["id"] == "invalid-utf8")
            .unwrap();
        assert_eq!(invalid["readable"], false);
        assert_eq!(invalid["configuredEnabled"], true);
        assert_eq!(invalid["error"]["code"], "agent_skill_read_failed");
        let oversized = catalog
            .iter()
            .find(|entry| entry["id"] == "oversized")
            .unwrap();
        assert_eq!(oversized["enabled"], false);
        assert_eq!(oversized["configuredEnabled"], false);
        assert_eq!(oversized["error"]["code"], "agent_skill_too_large");
        assert_eq!(
            read_from_roots(&roots, "oversized").await.unwrap_err().code,
            "agent_skill_too_large"
        );
        assert_eq!(
            read_from_roots(&roots, "invalid-utf8")
                .await
                .unwrap_err()
                .code,
            "agent_skill_read_failed"
        );
        update_from_roots(&roots, "valid", "Edited valid")
            .await
            .unwrap();
        update_from_roots(&roots, "repair", "Repaired managed instructions")
            .await
            .unwrap();
        let repaired = catalog_from_roots(roots, &overrides).await.unwrap();
        assert!(repaired
            .iter()
            .find(|entry| entry["id"] == "repair")
            .unwrap()["enabled"]
            .as_bool()
            .unwrap());
    }
}
