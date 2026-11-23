use std::path::PathBuf;

pub(crate) fn skill_description(text: &str) -> Option<String> {
    let mut lines = text.lines().map(str::trim).peekable();
    if lines.peek().copied() == Some("---") {
        lines.next();
        for line in &mut lines {
            if line == "---" {
                break;
            }
            if let Some((key, value)) = line.split_once(':') {
                if key.trim() == "description" {
                    let value = value.trim().trim_matches(['"', '\'']);
                    if !value.is_empty() {
                        return Some(value.to_string());
                    }
                }
            }
        }
    }
    lines
        .find(|line| !line.is_empty() && !line.starts_with('#') && *line != "---")
        .map(str::to_string)
}

pub(super) fn skill_read_error(error: std::io::Error) -> crate::domain::types::AppError {
    let code = if error.kind() == std::io::ErrorKind::InvalidData
        && error.to_string() == "Skill content exceeds 512 KiB."
    {
        "agent_skill_too_large"
    } else {
        "agent_skill_read_failed"
    };
    crate::domain::types::AppError::new(code, error.to_string())
}

pub(crate) async fn catalog_from_roots(
    roots: Vec<(PathBuf, bool)>,
    overrides: &std::collections::HashMap<String, bool>,
) -> Result<Vec<serde_json::Value>, crate::domain::types::AppError> {
    use super::skill_identity::inspect_skills;
    let mut result = Vec::new();
    for inspected in inspect_skills(&roots, overrides) {
        let enabled = inspected.enabled();
        let readable = inspected.readable();
        let configured_enabled = inspected.configured_enabled;
        let skill = inspected.skill;
        let (description, error) = match inspected.content {
            Ok(content) => (
                skill_description(&content).unwrap_or_else(|| "Clovy agent skill".into()),
                None,
            ),
            Err(error) => {
                let error = skill_read_error(error);
                let message = if error.code == "agent_skill_too_large" {
                    "Skill content exceeds 512 KiB."
                } else {
                    "Skill instructions could not be read as UTF-8 text."
                };
                (
                    format!("Unavailable skill: {message}"),
                    Some(serde_json::json!({"code":error.code,"message":message})),
                )
            }
        };
        result.push(serde_json::json!({"id":skill.id,"name":skill.id,"description":description,"source":if skill.managed {"managed"} else {"user_global"},"enabled":enabled,"configuredEnabled":configured_enabled,"readable":readable,"error":error,"editable":skill.managed,"root":skill.root}));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn catalog_preserves_first_line_and_frontmatter_descriptions() {
        let root = tempfile::tempdir().unwrap();
        for (id, content, _) in [
            ("one", "One line", "One line"),
            ("multi", "First line\nSecond line", "First line"),
            ("heading", "# Heading\nBody", "Body"),
            (
                "front",
                "---\nname: front\ndescription: 'Summary'\n---\nBody",
                "Summary",
            ),
            ("nodesc", "---\nname: nodesc\n---\n# Heading\nBody", "Body"),
        ] {
            let directory = root.path().join(id);
            std::fs::create_dir(&directory).unwrap();
            std::fs::write(directory.join("SKILL.md"), content).unwrap();
        }
        let catalog = catalog_from_roots(vec![(root.path().to_owned(), true)], &Default::default())
            .await
            .unwrap();
        for (id, expected) in [
            ("one", "One line"),
            ("multi", "First line"),
            ("heading", "Body"),
            ("front", "Summary"),
            ("nodesc", "Body"),
        ] {
            assert_eq!(
                catalog.iter().find(|entry| entry["id"] == id).unwrap()["description"],
                expected
            );
        }
    }
}
