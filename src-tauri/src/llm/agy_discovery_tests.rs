use super::*;
use std::process::Command;

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

fn host(root: &Path) -> HostContext {
    HostContext::new(
        root.canonicalize().unwrap(),
        &BTreeMap::from([
            (
                "HOME".into(),
                root.join("synthetic-home").display().to_string(),
            ),
            (
                "APPDATA".into(),
                root.join("synthetic-appdata").display().to_string(),
            ),
        ]),
    )
    .unwrap()
}

fn git(cwd: &Path, env: &BTreeMap<String, String>, args: &[&str]) -> String {
    // Git is a test oracle only. Neither the product nor this test invokes a
    // personal config, template or hook. All filesystem targets are synthetic.
    let vars = BTreeMap::from([
        ("PATH".into(), std::env::var("PATH").unwrap()),
        ("PATHEXT".into(), ".COM;.EXE;.BAT;.CMD".into()),
    ]);
    let program = crate::llm::shell_env::LoginEnv::from_vars(vars)
        .which("git")
        .expect("Git is required only for this invariant test");
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(env)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_CONFIG_COUNT", "0")
        .env("GIT_TERMINAL_PROMPT", "0");
    if cfg!(windows) {
        command.env("SystemRoot", std::env::var_os("SystemRoot").unwrap());
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn parent_repository(root: &Path) {
    let git = root.join(".git");
    for path in [
        "objects/info",
        "objects/pack",
        "refs/heads",
        "refs/tags",
        "hooks",
        "info",
    ] {
        std::fs::create_dir_all(git.join(path)).unwrap();
    }
    std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(
        git.join("config"),
        "[core]\nrepositoryformatversion = 0\nbare = false\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join(".agents")).unwrap();
    std::fs::write(
        root.join(".agents/hooks.json"),
        b"synthetic ancestor hook configuration",
    )
    .unwrap();
}

fn prepare(host: &HostContext, path: &str) -> Execution {
    match path {
        "chat" => host.chat("discovery-session").unwrap(),
        "one-shot" => host.one_shot().unwrap(),
        "login" => host.login().unwrap(),
        _ => unreachable!(),
    }
}

fn launch(root: &Path) -> McpLaunch {
    McpLaunch {
        command: std::env::current_exe().unwrap(),
        args: vec![
            "--dir".into(),
            root.join("mcp").display().to_string(),
            "--turn".into(),
            "synthetic-discovery-turn".into(),
        ],
    }
}

#[test]
fn f15_discovery_all_execution_paths_stop_at_their_own_repository() {
    let root = root();
    parent_repository(root.path());
    let parent_config = std::fs::read(root.path().join(".git/config")).unwrap();
    let parent_hook = std::fs::read(root.path().join(".agents/hooks.json")).unwrap();
    let host = host(root.path());
    let mut origin = None;
    for path in ["chat", "one-shot", "login"] {
        let mut execution = prepare(&host, path);
        let cwd = execution.cwd().canonicalize().unwrap();
        let home = execution.home().to_path_buf();
        std::fs::write(home.join("synthetic-native-auth"), b"native state retained").unwrap();
        if let Some(origin) = &origin {
            assert_eq!(execution.profile_id(), origin);
        } else {
            origin = Some(execution.profile_id().to_string());
        }
        let env = execution.environment(&BTreeMap::from([
            ("PATH".into(), std::env::var("PATH").unwrap()),
            (
                "GIT_DIR".into(),
                root.path().join(".git").display().to_string(),
            ),
            ("Git_Work_Tree".into(), root.path().display().to_string()),
            (
                "GIT_CONFIG_SYSTEM".into(),
                root.path().join("external-config").display().to_string(),
            ),
            (
                "GIT_COMMON_DIR".into(),
                root.path().join(".git").display().to_string(),
            ),
        ]));
        assert!(!env
            .keys()
            .any(|key| key.to_ascii_uppercase().starts_with("GIT_")));
        let observed = PathBuf::from(git(&cwd, &env, &["rev-parse", "--show-toplevel"]))
            .canonicalize()
            .unwrap();
        assert_eq!(observed, cwd, "{path} inherited the ancestor repository");
        execution.install_mcp(&launch(root.path())).unwrap();
        assert_eq!(
            git(
                &cwd,
                &env,
                &["status", "--porcelain", "--untracked-files=all"]
            ),
            "",
            "transport must be excluded from the empty repository"
        );
        execution.finish().unwrap();
        drop(execution);
        assert_eq!(cwd.exists(), path != "one-shot");
        assert_eq!(
            std::fs::read(home.join("synthetic-native-auth")).unwrap(),
            b"native state retained"
        );
    }
    let chat = host.chat("discovery-session").unwrap();
    assert_eq!(Some(chat.profile_id()), origin.as_deref());
    assert_eq!(
        std::fs::read(root.path().join(".git/config")).unwrap(),
        parent_config
    );
    assert_eq!(
        std::fs::read(root.path().join(".agents/hooks.json")).unwrap(),
        parent_hook
    );
}

#[test]
fn f15_discovery_oneshot_cleanup_preserves_unowned_workspace_data() {
    let root = root();
    let host = host(root.path());
    let execution = host.one_shot().unwrap();
    let cwd = execution.cwd().to_path_buf();
    let home = execution.home().to_path_buf();
    std::fs::write(
        home.join("synthetic-native-auth"),
        b"authentication is durable",
    )
    .unwrap();
    std::fs::write(cwd.join("unowned-data"), b"must not be recursively removed").unwrap();
    drop(execution);
    assert_eq!(
        std::fs::read(cwd.join("unowned-data")).unwrap(),
        b"must not be recursively removed"
    );
    assert_eq!(
        std::fs::read(home.join("synthetic-native-auth")).unwrap(),
        b"authentication is durable"
    );
}

#[test]
fn f15_discovery_preexisting_git_is_not_adopted_or_rewritten() {
    let root = root();
    let host = host(root.path());
    let profile = host.profile().unwrap();
    let directory = profile.root.child("preexisting-workspace").unwrap();
    directory.child(".agents").unwrap();
    parent_repository(directory.path());
    let config = directory.path().join(".git/config");
    let before = std::fs::read(&config).unwrap();
    let cwd = directory.path().to_path_buf();
    assert!(Execution::new(profile, directory, true).is_err());
    assert_eq!(std::fs::read(config).unwrap(), before);
    assert!(!cwd.join(".agents/mcp_config.json").exists());
}

#[test]
fn rf1_transport_crash_child() {
    let Some(path) = std::env::var_os("CLOVY_AGY_RF1_TEST_ROOT") else {
        return;
    };
    let root = PathBuf::from(path);
    let mut execution = host(&root).chat("rf1-session").unwrap();
    execution.install_mcp(&launch(&root)).unwrap();
    panic!("the requested crash point was not reached");
}

fn recover_crash(point: &str) {
    let root = root();
    let host = host(root.path());
    let execution = host.chat("rf1-session").unwrap();
    let auth = execution.home().join("synthetic-native-auth");
    let cwd = execution.cwd().to_path_buf();
    let origin = execution.profile_id().to_string();
    std::fs::write(
        &auth,
        b"native authentication survives interrupted publication",
    )
    .unwrap();
    drop(execution);
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "llm::agy::discovery_tests::rf1_transport_crash_child",
            "--nocapture",
        ])
        .env("CLOVY_AGY_RF1_TEST_ROOT", root.path())
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
    let agents = cwd.join(".agents");
    if point != "config-published" {
        assert!(
            !agents.join(CONFIG).exists(),
            "a partial config must never occupy the published name"
        );
    }
    if agents.join(RECEIPT).exists() {
        assert!(
            serde_json::from_slice::<Value>(&std::fs::read(agents.join(RECEIPT)).unwrap()).is_ok(),
            "the published receipt must always be complete"
        );
    }
    let unpublished: Vec<_> = std::fs::read_dir(&agents)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(".clovy-mcp-config-")
                || path
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with(".clovy-mcp-receipt-")
        })
        .map(|path| {
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    let has_receipt = agents.join(RECEIPT).exists();
    let mut recovered = host
        .chat("rf1-session")
        .expect("a private interrupted publication must not permanently block this session");
    assert_eq!(recovered.cwd(), cwd);
    assert_eq!(recovered.profile_id(), origin);
    assert!(!cwd.join(".agents/mcp_config.json").exists());
    recovered.install_mcp(&launch(root.path())).unwrap();
    recovered.finish().unwrap();
    drop(recovered);
    if !has_receipt {
        // Without a receipt there is no authority to delete a staged name.
        // Retaining it must not prevent this same session from running again.
        for (path, bytes) in unpublished {
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
    }
    assert_eq!(
        std::fs::read(auth).unwrap(),
        b"native authentication survives interrupted publication"
    );
}

#[test]
fn rf1_config_creation_crash_can_resume_same_session() {
    recover_crash("config-created");
}
#[test]
fn rf1_partial_receipt_crash_can_resume_same_session() {
    recover_crash("receipt-partial");
}
#[test]
fn rf1_partial_config_crash_can_resume_same_session() {
    recover_crash("config-partial");
}

#[test]
fn rf1_complete_receipt_before_config_publication_can_resume() {
    recover_crash("receipt-published");
}

#[test]
fn rf1_other_atomic_publication_crash_points_can_resume() {
    for point in [
        "config-ready",
        "receipt-created",
        "receipt-ready",
        "config-published",
    ] {
        recover_crash(point);
    }
}

#[test]
fn rf1_legacy_identified_partial_config_is_recovered_without_touching_auth() {
    let root = root();
    let host = host(root.path());
    let mut execution = host.chat("legacy-partial").unwrap();
    execution.install_mcp(&launch(root.path())).unwrap();
    let cwd = execution.cwd().to_path_buf();
    let auth = execution.home().join("synthetic-native-auth");
    std::fs::write(&auth, b"legacy auth retained").unwrap();
    let path = cwd.join(".agents").join(RECEIPT);
    let mut receipt: Receipt = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    receipt.version = 1;
    receipt.candidate = None;
    let entry = receipt.config["mcpServers"]
        .as_object_mut()
        .unwrap()
        .remove(&receipt.server)
        .unwrap();
    receipt.server = format!("clovy_turn_{}", Uuid::new_v4().simple());
    receipt.config = json!({"mcpServers":{(receipt.server.clone()):entry}});
    std::fs::write(&path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    let config = cwd.join(".agents").join(CONFIG);
    let bytes = serde_json::to_vec_pretty(&receipt.config).unwrap();
    std::fs::write(&config, &bytes[..bytes.len() / 2]).unwrap();
    execution.finish().unwrap();
    drop(execution);
    assert!(!config.exists());
    let _resumed = host.chat("legacy-partial").unwrap();
    assert_eq!(std::fs::read(auth).unwrap(), b"legacy auth retained");
}

#[test]
fn rf1_missing_malformed_or_foreign_publication_proof_is_retained_and_refused() {
    for case in ["missing-receipt", "partial-receipt", "foreign-candidate"] {
        let root = root();
        let host = host(root.path());
        let mut execution = host.chat("foreign-publication").unwrap();
        execution.install_mcp(&launch(root.path())).unwrap();
        let cwd = execution.cwd().to_path_buf();
        let config = cwd.join(".agents").join(CONFIG);
        let before = std::fs::read(&config).unwrap();
        let receipt_path = cwd.join(".agents").join(RECEIPT);
        match case {
            "missing-receipt" => std::fs::remove_file(&receipt_path).unwrap(),
            "partial-receipt" => std::fs::write(&receipt_path, b"{\"version\":").unwrap(),
            _ => {
                let receipt: Receipt =
                    serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
                execution
                    .directory
                    .as_ref()
                    .unwrap()
                    .existing_child(".agents")
                    .unwrap()
                    .write_new(receipt.candidate.as_deref().unwrap(), b"foreign candidate")
                    .unwrap();
            }
        }
        assert!(execution.finish().is_err());
        drop(execution);
        assert_eq!(
            std::fs::read(config).unwrap(),
            before,
            "validate every unit before removing any"
        );
        assert!(host.chat("foreign-publication").is_err());
    }
}

#[test]
fn rf1_atomic_publication_never_replaces_an_existing_unit() {
    let root = root();
    let directory = Directory::root(&root.path().canonicalize().unwrap()).unwrap();
    let identity = directory
        .write_new("existing", b"foreign content retained")
        .unwrap();
    assert!(directory
        .publish_new("existing", b"must not replace", "config")
        .is_err());
    let (bytes, current) = directory.read("existing").unwrap().unwrap();
    assert_eq!(bytes, b"foreign content retained");
    assert_eq!(current, identity);
}

#[test]
fn f15_discovery_migrates_owned_chat_workspace_without_changing_profile_origin() {
    let root = root();
    let host = host(root.path());
    let profile = host.profile().unwrap();
    let origin = profile.id.clone();
    let auth = profile.home.path().join("synthetic-native-auth");
    std::fs::write(&auth, b"existing authentication").unwrap();
    // The previous delivery's durable workspace had a lease and .agents,
    // without a local repository. No SDK state is copied or reset.
    let key = format!("{:x}", Sha256::digest(b"migration-session"));
    let workspace = profile
        .root
        .child("workspaces")
        .unwrap()
        .child("sessions")
        .unwrap()
        .child(&key)
        .unwrap();
    drop(private::lease(&workspace).unwrap());
    let agents = workspace.child(".agents").unwrap();
    agents
        .write_new("keep.txt", b"existing unrelated data")
        .unwrap();
    let cwd = workspace.path().to_path_buf();
    drop(agents);
    drop(workspace);
    drop(profile);
    let execution = host.chat("migration-session").unwrap();
    assert_eq!(execution.cwd(), cwd);
    assert_eq!(execution.profile_id(), origin);
    assert_eq!(std::fs::read(auth).unwrap(), b"existing authentication");
    assert_eq!(
        std::fs::read(cwd.join(".agents/keep.txt")).unwrap(),
        b"existing unrelated data"
    );
    assert_eq!(
        PathBuf::from(git(
            &cwd,
            &execution.environment(&BTreeMap::new()),
            &["rev-parse", "--show-toplevel"]
        ))
        .canonicalize()
        .unwrap(),
        cwd
    );
}

#[test]
fn f15_discovery_replaced_or_unknown_git_units_are_retained_on_all_paths() {
    for path in ["chat", "one-shot", "login"] {
        for replacement in [true, false] {
            let root = root();
            let host = host(root.path());
            let mut execution = prepare(&host, path);
            let cwd = execution.cwd().to_path_buf();
            let auth = execution.home().join("synthetic-native-auth");
            std::fs::write(&auth, b"authentication survives refusal").unwrap();
            let target = if replacement {
                let config = cwd.join(".git/config");
                let bytes = std::fs::read(&config).unwrap();
                let git = execution
                    .directory
                    .as_ref()
                    .unwrap()
                    .existing_child(".git")
                    .unwrap();
                git.write_new("replacement", &bytes).unwrap();
                std::fs::rename(cwd.join(".git/replacement"), &config).unwrap();
                config
            } else {
                let target = cwd.join(".git/hooks/foreign-hook");
                std::fs::write(&target, b"unknown hook must not be run or removed").unwrap();
                target
            };
            let before = std::fs::read(&target).unwrap();
            assert!(execution.install_mcp(&launch(root.path())).is_err());
            assert!(!cwd.join(".agents/mcp_config.json").exists());
            assert!(
                execution.finish().is_err(),
                "changed discovery boundary must remain diagnosable"
            );
            drop(execution);
            assert_eq!(std::fs::read(target).unwrap(), before);
            assert_eq!(
                std::fs::read(auth).unwrap(),
                b"authentication survives refusal"
            );
            if path == "chat" {
                assert!(host.chat("discovery-session").is_err());
            }
        }
    }
}

#[test]
fn f15_discovery_cleanup_reports_changed_boundary_after_removing_owned_transport() {
    let root = root();
    let host = host(root.path());
    let mut execution = host.chat("changed-after-start").unwrap();
    execution.install_mcp(&launch(root.path())).unwrap();
    let cwd = execution.cwd().to_path_buf();
    let auth = execution.home().join("synthetic-native-auth");
    std::fs::write(&auth, b"keep native authentication").unwrap();
    std::fs::write(cwd.join(".git/hooks/unknown"), b"unowned metadata").unwrap();
    let error = execution.finish().unwrap_err();
    assert!(error.to_string().contains("discovery boundary"));
    assert!(!cwd.join(".agents/mcp_config.json").exists());
    assert!(!cwd.join(".agents").join(RECEIPT).exists());
    drop(execution);
    assert_eq!(
        std::fs::read(cwd.join(".git/hooks/unknown")).unwrap(),
        b"unowned metadata"
    );
    assert_eq!(std::fs::read(auth).unwrap(), b"keep native authentication");
}

#[cfg(unix)]
#[test]
fn f15_discovery_symlink_and_unsafe_metadata_never_publish_authority() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    for path in ["chat", "one-shot", "login"] {
        for link in [true, false] {
            let root = root();
            let foreign = root.path().join("foreign");
            std::fs::create_dir(&foreign).unwrap();
            std::fs::write(foreign.join("keep"), b"external data").unwrap();
            let host = host(root.path());
            let mut execution = prepare(&host, path);
            let cwd = execution.cwd().to_path_buf();
            let auth = execution.home().join("synthetic-native-auth");
            std::fs::write(&auth, b"native authentication").unwrap();
            let config = cwd.join(".git/config");
            if link {
                // Remove only this test's freshly created, known private unit.
                std::fs::remove_file(&config).unwrap();
                symlink(foreign.join("keep"), &config).unwrap();
            } else {
                std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o644)).unwrap();
            }
            let before = std::fs::symlink_metadata(&config).unwrap();
            assert!(execution.install_mcp(&launch(root.path())).is_err());
            assert!(!cwd.join(".agents/mcp_config.json").exists());
            drop(execution);
            let after = std::fs::symlink_metadata(&config).unwrap();
            assert_eq!(before.permissions().mode(), after.permissions().mode());
            assert_eq!(after.file_type().is_symlink(), link);
            assert_eq!(
                std::fs::read(foreign.join("keep")).unwrap(),
                b"external data"
            );
            assert_eq!(std::fs::read(auth).unwrap(), b"native authentication");
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn f15_discovery_existing_git_acl_is_rejected_without_repair() {
    let root = root();
    let host = host(root.path());
    let execution = host.chat("discovery-session").unwrap();
    let cwd = execution.cwd().to_path_buf();
    drop(execution);
    let config = cwd.join(".git/config");
    assert!(Command::new("/bin/chmod")
        .args(["+a", "everyone allow read,readattr,readsecurity"])
        .arg(&config)
        .status()
        .unwrap()
        .success());
    let listing = || {
        Command::new("/bin/ls")
            .arg("-le")
            .arg(&config)
            .output()
            .unwrap()
            .stdout
    };
    let before = listing();
    assert!(host.chat("discovery-session").is_err());
    assert_eq!(listing(), before);
    assert!(!cwd.join(".agents/mcp_config.json").exists());
}
