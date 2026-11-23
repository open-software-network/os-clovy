use std::{
    collections::{BTreeSet, HashMap},
    fs, io,
    path::{Path, PathBuf},
};

pub const CLOVY_OBSIDIAN_SKILL_ID: &str = "clovy-obsidian";
/// The id a profile copied from a June-era install may still store or hold on
/// disk. It is read and mapped to the Clovy id, never written.
pub const LEGACY_OBSIDIAN_SKILL_ID: &str = "june-obsidian";

/// Only names that can also be addressed through the skill API are discoverable.
pub fn valid_skill_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// Resolve symlinks before reading so a skill cannot escape its accepted root.
pub fn contained_skill_file(root: &Path, id: &str) -> Option<PathBuf> {
    if !valid_skill_id(id) {
        return None;
    }
    let root = root.canonicalize().ok()?;
    let file = skill_file(&root, id).canonicalize().ok()?;
    (file.starts_with(&root) && file.is_file()).then_some(file)
}

pub fn canonical_skill_id(skill_id: &str) -> &str {
    if skill_id == LEGACY_OBSIDIAN_SKILL_ID {
        CLOVY_OBSIDIAN_SKILL_ID
    } else {
        skill_id
    }
}

pub fn canonical_skill_ids<'a>(skill_ids: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    skill_ids
        .into_iter()
        .map(canonical_skill_id)
        .map(str::to_string)
        .collect()
}

/// Project stored alias settings once. An explicit canonical row always wins,
/// independently of database ordering; legacy rows remain read-only inputs.
pub fn skill_enabled_overrides(
    settings: impl IntoIterator<Item = (String, bool)>,
) -> HashMap<String, bool> {
    let mut result = HashMap::new();
    let mut canonical_rows = BTreeSet::new();
    for (id, enabled) in settings {
        let canonical = canonical_skill_id(&id).to_string();
        if id == canonical {
            canonical_rows.insert(canonical.clone());
            result.insert(canonical, enabled);
        } else if !canonical_rows.contains(&canonical) {
            result.insert(canonical, enabled);
        }
    }
    result
}

pub fn read_skill_ids(skill_id: &str) -> Vec<&str> {
    let canonical = canonical_skill_id(skill_id);
    if canonical == CLOVY_OBSIDIAN_SKILL_ID {
        vec![CLOVY_OBSIDIAN_SKILL_ID, LEGACY_OBSIDIAN_SKILL_ID]
    } else {
        vec![canonical]
    }
}

pub fn write_skill_ids(skill_id: &str) -> Vec<&str> {
    vec![canonical_skill_id(skill_id)]
}

pub fn skill_file(root: &Path, skill_id: &str) -> PathBuf {
    root.join(skill_id).join("SKILL.md")
}

/// Reconciles the managed Obsidian skill before the bundled resource is seeded:
/// a June-era copy is read once into the Clovy id and never written again.
pub fn reconcile_managed_obsidian_skill(root: &Path) -> io::Result<()> {
    if contained_skill_file(root, CLOVY_OBSIDIAN_SKILL_ID).is_some() {
        return Ok(());
    }
    let Some(legacy) = contained_skill_file(root, LEGACY_OBSIDIAN_SKILL_ID) else {
        return Ok(());
    };
    let content = read_skill_content(&legacy)?;
    update_managed_skill(root, CLOVY_OBSIDIAN_SKILL_ID, &content)
}

pub fn skill_content_for_id(content: &str, skill_id: &str) -> String {
    let mut lines = content.split_inclusive('\n').peekable();
    let Some(first) = lines.next() else {
        return format!("---\nname: {skill_id}\n---\n");
    };
    if first.trim_end_matches(['\r', '\n']) != "---" {
        return format!("---\nname: {skill_id}\n---\n\n{content}");
    }

    let newline = if first.ends_with("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut output = first.to_string();
    let mut found_name = false;
    let mut closed_frontmatter = false;
    for line in lines {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if !closed_frontmatter && trimmed.trim() == "---" {
            if !found_name {
                output.push_str(&format!("name: {skill_id}{newline}"));
            }
            output.push_str(line);
            closed_frontmatter = true;
            continue;
        }
        if !closed_frontmatter
            && trimmed
                .split_once(':')
                .is_some_and(|(key, _)| key.trim() == "name")
        {
            let indentation = &trimmed[..trimmed.len() - trimmed.trim_start().len()];
            output.push_str(indentation);
            output.push_str("name: ");
            output.push_str(skill_id);
            if line.ends_with("\r\n") {
                output.push_str("\r\n");
            } else if line.ends_with('\n') {
                output.push('\n');
            }
            found_name = true;
        } else {
            output.push_str(line);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_accepts_only_addressable_names_and_contained_files() {
        let root = tempfile::tempdir().unwrap();
        for id in [
            "research",
            "foo-backup",
            ".hidden",
            "notes.bak",
            "notes.old",
            "notes~",
        ] {
            fs::create_dir(root.path().join(id)).unwrap();
            fs::write(skill_file(root.path(), id), "instructions").unwrap();
        }
        assert!(contained_skill_file(root.path(), "research").is_some());
        assert!(contained_skill_file(root.path(), "foo-backup").is_some());
        for id in [
            ".hidden",
            "notes.bak",
            "notes.old",
            "notes~",
            "../outside",
            "",
        ] {
            assert!(contained_skill_file(root.path(), id).is_none(), "{id}");
        }
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            fs::write(outside.path().join("SKILL.md"), "private").unwrap();
            std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
            assert!(contained_skill_file(root.path(), "escape").is_none());
        }
    }

    #[test]
    fn a_june_era_skill_is_read_once_into_the_clovy_id() {
        let root = tempfile::tempdir().expect("managed skill root");
        let legacy = skill_file(root.path(), LEGACY_OBSIDIAN_SKILL_ID);
        fs::create_dir_all(legacy.parent().expect("legacy parent")).expect("legacy directory");
        let original = "---\nname: june-obsidian\ndescription: User description\ncustom: keep-me\n---\n\nUser-edited body\n";
        fs::write(&legacy, original).expect("legacy skill");

        reconcile_managed_obsidian_skill(root.path()).expect("read legacy skill");

        let canonical = fs::read_to_string(skill_file(root.path(), CLOVY_OBSIDIAN_SKILL_ID))
            .expect("canonical skill");
        assert!(canonical.contains("name: clovy-obsidian"));
        assert!(canonical.contains("custom: keep-me"));
        assert!(canonical.ends_with("User-edited body\n"));
        assert_eq!(
            fs::read_to_string(&legacy).expect("legacy untouched"),
            original
        );
    }

    #[test]
    fn the_clovy_skill_is_never_mirrored_back() {
        let root = tempfile::tempdir().expect("managed skill root");
        let canonical = skill_file(root.path(), CLOVY_OBSIDIAN_SKILL_ID);
        fs::create_dir_all(canonical.parent().expect("canonical parent"))
            .expect("canonical directory");
        fs::write(&canonical, "---\nname: clovy-obsidian\n---\n\ninstructions")
            .expect("canonical skill");

        reconcile_managed_obsidian_skill(root.path()).expect("reconcile");

        assert!(!skill_file(root.path(), LEGACY_OBSIDIAN_SKILL_ID).exists());
        assert_eq!(
            write_skill_ids(LEGACY_OBSIDIAN_SKILL_ID),
            vec![CLOVY_OBSIDIAN_SKILL_ID]
        );
    }

    #[test]
    fn stored_june_era_ids_read_as_the_clovy_id() {
        assert_eq!(
            canonical_skill_ids([LEGACY_OBSIDIAN_SKILL_ID, "research"]),
            BTreeSet::from([CLOVY_OBSIDIAN_SKILL_ID.to_string(), "research".to_string()])
        );
    }
}

pub const MAX_SKILL_BYTES: usize = 512 * 1024;

/// One canonical identity, resolved once with managed roots preceding global roots.
#[derive(Debug, Clone)]
pub struct DiscoveredSkill {
    pub id: String,
    pub path: PathBuf,
    pub root: PathBuf,
    pub managed: bool,
}

/// Resolve future storage through its existing ancestors before any directory
/// is created. Root aliases must not turn global read-only storage writable.
fn resolved_storage_path(path: &Path) -> Option<PathBuf> {
    let mut ancestor = path;
    let mut suffix = Vec::new();
    loop {
        if let Ok(mut resolved) = ancestor.canonicalize() {
            for component in suffix.iter().rev() {
                resolved.push(component);
            }
            return Some(resolved);
        }
        suffix.push(ancestor.file_name()?.to_os_string());
        ancestor = ancestor.parent()?;
    }
}

pub fn managed_root_is_writable(root: &Path, roots: &[(PathBuf, bool)]) -> bool {
    let Some(root) = resolved_storage_path(root) else {
        return false;
    };
    !roots
        .iter()
        .filter(|(_, managed)| !managed)
        .filter_map(|(global, _)| resolved_storage_path(global))
        .any(|global| root.starts_with(&global) || global.starts_with(&root))
}

pub fn discover_skills(roots: &[(PathBuf, bool)]) -> Vec<DiscoveredSkill> {
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    let global_roots = roots
        .iter()
        .filter(|(_, managed)| !managed)
        .filter_map(|(root, _)| root.canonicalize().ok())
        .collect::<Vec<_>>();
    for (root, managed) in roots {
        let writable = *managed && managed_root_is_writable(root, roots);
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        let mut names = entries
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect::<Vec<_>>();
        names.sort();
        // Prefer a canonical directory over its legacy alias in each root.
        names.sort_by_key(|name| canonical_skill_id(name) != name);
        for stored_id in names {
            let Some(path) = contained_skill_file(root, &stored_id) else {
                continue;
            };
            let id = canonical_skill_id(&stored_id).to_string();
            if seen.insert(id.clone()) {
                let editable = writable && !global_roots.iter().any(|root| path.starts_with(root));
                result.push(DiscoveredSkill {
                    id,
                    path,
                    root: root.clone(),
                    managed: editable,
                });
            }
        }
    }
    result.sort_by(|left, right| left.id.cmp(&right.id));
    result
}

/// Administration keeps each discovered identity even when its content cannot
/// be loaded. Such an entry never becomes an active skill.
#[derive(Debug)]
pub struct InspectedSkill {
    pub skill: DiscoveredSkill,
    pub configured_enabled: bool,
    pub content: io::Result<String>,
}
impl InspectedSkill {
    pub fn readable(&self) -> bool {
        self.content.is_ok()
    }
    pub fn enabled(&self) -> bool {
        self.configured_enabled && self.readable()
    }
}

pub fn inspect_skills(
    roots: &[(PathBuf, bool)],
    overrides: &HashMap<String, bool>,
) -> Vec<InspectedSkill> {
    discover_skills(roots)
        .into_iter()
        .map(|skill| {
            let configured_enabled = overrides.get(&skill.id).copied().unwrap_or(true);
            let content = read_skill_content(&skill.path);
            InspectedSkill {
                skill,
                configured_enabled,
                content,
            }
        })
        .collect()
}

/// Bound reads allocate at most the accepted content limit, including when the
/// file grows after metadata was inspected.
pub fn read_skill_content(path: &Path) -> io::Result<String> {
    use io::Read;
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take((MAX_SKILL_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_SKILL_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Skill content exceeds 512 KiB.",
        ));
    }
    String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Atomic replacement remains anchored to the approved directory object.
pub fn update_managed_skill(root: &Path, skill_id: &str, content: &str) -> io::Result<()> {
    if !valid_skill_id(skill_id) || content.len() > MAX_SKILL_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid skill id or content size.",
        ));
    }
    let id = canonical_skill_id(skill_id);
    let canonical_root = root.canonicalize()?;
    if !read_skill_ids(id)
        .iter()
        .any(|read_id| contained_skill_file(root, read_id).is_some())
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Managed skill was not found.",
        ));
    }
    let directory = super::skill_writer::AnchoredSkillDirectory::open(&canonical_root, id)?;
    #[cfg(test)]
    super::skill_writer::after_bind();
    let content = skill_content_for_id(content, id);
    if content.len() > MAX_SKILL_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Skill content exceeds 512 KiB.",
        ));
    }
    directory.replace(&content)
}

#[cfg(test)]
mod discovery_regressions {
    use super::*;
    #[test]
    fn managed_precedence_canonical_aliases_and_containment_share_identity() {
        let managed = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        for (root, id, text) in [
            (managed.path(), "research", "managed"),
            (global.path(), "research", "global"),
            (managed.path(), "june-obsidian", "legacy"),
            (managed.path(), "clovy-obsidian", "canonical"),
            (managed.path(), "foo-backup", "valid"),
            (managed.path(), ".creative.backup-1", "hidden"),
            (managed.path(), "research.old", "backup"),
        ] {
            fs::create_dir_all(root.join(id)).unwrap();
            fs::write(skill_file(root, id), text).unwrap();
        }
        let catalog =
            discover_skills(&[(managed.path().into(), true), (global.path().into(), false)]);
        assert_eq!(
            catalog
                .iter()
                .map(|skill| skill.id.as_str())
                .collect::<Vec<_>>(),
            ["clovy-obsidian", "foo-backup", "research"]
        );
        assert!(catalog.iter().all(|skill| skill.managed));
        assert_eq!(read_skill_content(&catalog[0].path).unwrap(), "canonical");
        update_managed_skill(managed.path(), "research", "Edited").unwrap();
        assert!(read_skill_content(&catalog[2].path)
            .unwrap()
            .ends_with("Edited"));
        assert_eq!(
            fs::read_to_string(skill_file(global.path(), "research")).unwrap(),
            "global"
        );
        assert!(skill_file(managed.path(), ".creative.backup-1").exists());
        assert!(
            update_managed_skill(managed.path(), "research", &"x".repeat(MAX_SKILL_BYTES + 1))
                .is_err()
        );
    }
    #[cfg(unix)]
    #[test]
    fn escaped_skill_symlinks_never_read_or_write_external_files() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("SKILL.md"), "private").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
        assert!(discover_skills(&[(root.path().into(), true)]).is_empty());
        assert!(update_managed_skill(root.path(), "escape", "Denied").is_err());
        assert_eq!(
            fs::read_to_string(outside.path().join("SKILL.md")).unwrap(),
            "private"
        );
        let alias = root.path().join("global-alias");
        std::os::unix::fs::symlink(outside.path(), &alias).unwrap();
        fs::create_dir(outside.path().join("global-skill")).unwrap();
        fs::write(outside.path().join("global-skill/SKILL.md"), "global").unwrap();
        assert!(
            discover_skills(&[(alias, true), (outside.path().into(), false)])
                .iter()
                .all(|skill| !skill.managed)
        );
    }
}

#[cfg(test)]
mod catalog_isolation_regressions {
    use super::*;
    #[test]
    fn canonical_overrides_win_over_legacy_rows_in_both_orders() {
        for rows in [
            vec![
                (CLOVY_OBSIDIAN_SKILL_ID.into(), false),
                (LEGACY_OBSIDIAN_SKILL_ID.into(), true),
            ],
            vec![
                (LEGACY_OBSIDIAN_SKILL_ID.into(), true),
                (CLOVY_OBSIDIAN_SKILL_ID.into(), false),
            ],
        ] {
            assert_eq!(
                skill_enabled_overrides(rows).get(CLOVY_OBSIDIAN_SKILL_ID),
                Some(&false)
            );
        }
    }
    #[test]
    fn unreadable_and_oversized_entries_are_isolated_and_never_active() {
        let root = tempfile::tempdir().unwrap();
        for id in ["valid", "oversized", "invalid-utf8"] {
            fs::create_dir(root.path().join(id)).unwrap();
        }
        fs::write(skill_file(root.path(), "valid"), "Valid instructions").unwrap();
        fs::write(
            skill_file(root.path(), "oversized"),
            vec![b'x'; MAX_SKILL_BYTES + 1],
        )
        .unwrap();
        fs::write(skill_file(root.path(), "invalid-utf8"), [0xff, 0xfe]).unwrap();
        let overrides = HashMap::from([("oversized".into(), false)]);
        let catalog = inspect_skills(&[(root.path().into(), false)], &overrides);
        assert_eq!(catalog.len(), 3);
        assert_eq!(
            catalog
                .iter()
                .filter(|entry| entry.enabled())
                .map(|entry| entry.skill.id.as_str())
                .collect::<Vec<_>>(),
            ["valid"]
        );
        assert!(
            !catalog
                .iter()
                .find(|entry| entry.skill.id == "oversized")
                .unwrap()
                .configured_enabled
        );
        assert!(
            catalog
                .iter()
                .find(|entry| entry.skill.id == "invalid-utf8")
                .unwrap()
                .configured_enabled
        );
        assert!(catalog
            .iter()
            .filter(|entry| entry.skill.id != "valid")
            .all(|entry| !entry.readable()));
    }
    #[cfg(unix)]
    #[test]
    fn managed_alias_cannot_admit_startup_writes_into_global_storage() {
        let root = tempfile::tempdir().unwrap();
        let global = root.path().join("global");
        fs::create_dir_all(global.join(LEGACY_OBSIDIAN_SKILL_ID)).unwrap();
        fs::write(
            skill_file(&global, LEGACY_OBSIDIAN_SKILL_ID),
            "Legacy global instructions",
        )
        .unwrap();
        let managed = root.path().join("managed");
        std::os::unix::fs::symlink(&global, &managed).unwrap();
        assert!(!managed_root_is_writable(
            &managed,
            &[(managed.clone(), true), (global.clone(), false)]
        ));
        assert!(!managed_root_is_writable(
            &global.join("future-managed"),
            &[(global.clone(), false)]
        ));
        assert!(!skill_file(&global, CLOVY_OBSIDIAN_SKILL_ID).exists());
        assert_eq!(
            fs::read_to_string(skill_file(&global, LEGACY_OBSIDIAN_SKILL_ID)).unwrap(),
            "Legacy global instructions"
        );
    }
}
