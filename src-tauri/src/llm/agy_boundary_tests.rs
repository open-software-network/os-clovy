use super::*;

fn root() -> tempfile::TempDir {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap()
    }
    #[cfg(windows)]
    {
        tempfile::tempdir().unwrap()
    }
}

fn host(path: &Path) -> HostContext {
    HostContext::new(
        path.canonicalize().unwrap(),
        &BTreeMap::from([
            (
                "HOME".into(),
                path.join("synthetic-host-home").display().to_string(),
            ),
            (
                "APPDATA".into(),
                path.join("synthetic-host-appdata").display().to_string(),
            ),
        ]),
    )
    .unwrap()
}

fn snapshot(path: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn read(root: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let metadata = std::fs::symlink_metadata(path).unwrap();
        assert!(
            !metadata.file_type().is_symlink(),
            "this snapshot covers only the test's own generated staging"
        );
        if metadata.is_dir() {
            files.insert(path.strip_prefix(root).unwrap().to_path_buf(), Vec::new());
            for entry in std::fs::read_dir(path).unwrap() {
                read(root, &entry.unwrap().path(), files);
            }
        } else {
            files.insert(
                path.strip_prefix(root).unwrap().to_path_buf(),
                std::fs::read(path).unwrap(),
            );
        }
    }
    let mut files = BTreeMap::new();
    read(path, path, &mut files);
    files
}

#[test]
fn rf2_boundary_crash_child() {
    let Some(path) = std::env::var_os("CLOVY_AGY_RF2_TEST_ROOT") else {
        return;
    };
    host(&PathBuf::from(path)).chat("rf2-session").unwrap();
    panic!("requested boundary crash point was not reached");
}

fn recover_crash(point: &str) {
    let root = root();
    let host = host(root.path());
    let profile = host.profile().unwrap();
    let origin = profile.id.clone();
    let auth = profile.home.path().join("synthetic-native-auth");
    std::fs::write(&auth, b"existing native authentication").unwrap();
    // Start before the first Agy context, with the same durable session path.
    let key = format!("{:x}", Sha256::digest(b"rf2-session"));
    let directory = profile
        .root
        .child("workspaces")
        .unwrap()
        .child("sessions")
        .unwrap()
        .child(&key)
        .unwrap();
    let cwd = directory.path().to_path_buf();
    drop(directory);
    drop(profile);
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "llm::agy::boundary_tests::rf2_boundary_crash_child",
            "--nocapture",
        ])
        .env("CLOVY_AGY_RF2_TEST_ROOT", root.path())
        .env("CLOVY_AGY_RF1_CRASH_POINT", point)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(87),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        cwd.join(".git").exists(),
        point == "boundary-published",
        "only a complete unit may occupy .git"
    );
    let staging: Vec<_> = std::fs::read_dir(&cwd)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(".clovy-workspace-")
        })
        .map(|path| {
            let before = snapshot(&path);
            (path, before)
        })
        .collect();
    let mut execution = host
        .chat("rf2-session")
        .expect("own interrupted boundary creation must not block the same session");
    assert_eq!(execution.cwd(), cwd);
    assert_eq!(execution.profile_id(), origin);
    execution.validate().unwrap();
    execution
        .install_mcp(&McpLaunch {
            command: std::env::current_exe().unwrap(),
            args: vec![
                "--dir".into(),
                root.path().join("mcp").display().to_string(),
                "--turn".into(),
                "synthetic-rf2-turn".into(),
            ],
        })
        .unwrap();
    assert!(cwd.join(".agents/mcp_config.json").exists());
    execution.finish().unwrap();
    drop(execution);
    let next = host.chat("rf2-session").unwrap();
    assert_eq!(next.profile_id(), origin);
    assert_eq!(
        std::fs::read(auth).unwrap(),
        b"existing native authentication"
    );
    for (path, before) in staging {
        assert_eq!(
            snapshot(&path),
            before,
            "unpublished staging is retained, never adopted or removed by its name"
        );
    }
}

#[test]
fn rf2_root_creation_crash_can_resume() {
    recover_crash("boundary-root-created");
}
#[test]
fn rf2_directory_creation_crash_can_resume() {
    recover_crash("boundary-directories-created");
}
#[test]
fn rf2_partial_git_file_crash_can_resume() {
    recover_crash("boundary-file-partial");
}
#[test]
fn rf2_partial_receipt_crash_can_resume() {
    recover_crash("boundary-receipt-partial");
}
#[test]
fn rf2_receipt_publication_crash_can_resume() {
    recover_crash("boundary-receipt-published");
}
#[test]
fn rf2_boundary_publication_crash_can_resume() {
    recover_crash("boundary-published");
}

#[test]
fn rf2_complete_legacy_boundary_is_not_rewritten_and_keeps_profile_origin() {
    let root = root();
    let host = host(root.path());
    let execution = host.chat("legacy-boundary").unwrap();
    let cwd = execution.cwd().to_path_buf();
    let origin = execution.profile_id().to_string();
    drop(execution);
    let workspace = Directory::root(&cwd).unwrap();
    let git = workspace.existing_child(".git").unwrap();
    let (bytes, identity) = git.read(BOUNDARY_RECEIPT).unwrap().unwrap();
    let mut proof: BoundaryProof = serde_json::from_slice(&bytes).unwrap();
    proof.version = 1;
    workspace
        .publish_new(
            BOUNDARY_RECEIPT,
            &serde_json::to_vec(&proof).unwrap(),
            "legacy-boundary-test",
        )
        .unwrap();
    git.remove(BOUNDARY_RECEIPT, &identity).unwrap();
    let before = snapshot(&cwd.join(".git"));
    let receipt = std::fs::read(cwd.join(BOUNDARY_RECEIPT)).unwrap();
    let execution = host.chat("legacy-boundary").unwrap();
    assert_eq!(execution.profile_id(), origin);
    assert!(!execution.boundary.embedded);
    assert_eq!(snapshot(&cwd.join(".git")), before);
    assert_eq!(std::fs::read(cwd.join(BOUNDARY_RECEIPT)).unwrap(), receipt);
}

#[test]
fn rf2_missing_or_invalid_published_proof_is_preserved_and_refused() {
    for missing in [true, false] {
        let root = root();
        let host = host(root.path());
        let execution = host.chat("foreign-boundary").unwrap();
        let cwd = execution.cwd().to_path_buf();
        drop(execution);
        let receipt = cwd.join(".git").join(BOUNDARY_RECEIPT);
        if missing {
            std::fs::remove_file(&receipt).unwrap();
        } else {
            std::fs::write(&receipt, b"{\"version\":").unwrap();
        }
        let before = snapshot(&cwd.join(".git"));
        assert!(host.chat("foreign-boundary").is_err());
        assert_eq!(snapshot(&cwd.join(".git")), before);
    }
}

#[test]
fn rf2_atomic_directory_publication_does_not_replace_a_foreign_destination() {
    let root = root();
    let workspace = Directory::root(&root.path().canonicalize().unwrap()).unwrap();
    let candidate = workspace.new_child("candidate").unwrap();
    candidate.write_new("keep", b"own staged data").unwrap();
    let identity = candidate.identity().unwrap();
    drop(candidate);
    let foreign = workspace.new_child(".git").unwrap();
    foreign.write_new("keep", b"foreign destination").unwrap();
    drop(foreign);
    let staged = snapshot(&workspace.path().join("candidate"));
    let before = snapshot(&workspace.path().join(".git"));
    assert!(workspace
        .publish_directory("candidate", ".git", &identity)
        .is_err());
    assert_eq!(snapshot(&workspace.path().join(".git")), before);
    assert_eq!(snapshot(&workspace.path().join("candidate")), staged);
}
