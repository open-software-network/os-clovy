//! GitHub via the GitHub CLI, with the fake `gh` and fake `api.github.com`
//! of [`super::super::github_cli_fixtures`].

use super::*;
use crate::connectors::github_cli_fixtures::*;
use crate::connectors::{policy, store, ConnectorAccountStatus, ConnectorProvider};
use std::path::PathBuf;

fn prefs_dir() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    (dir, path)
}

async fn app_account(repos: &crate::db::repositories::Repositories) {
    repos
        .upsert_connector_account(
            "999",
            ConnectorProvider::Github.as_str(),
            "app-user",
            &[policy::GITHUB_READ.to_string()],
            ConnectorAccountStatus::Connected.as_str(),
            r#"{"actorLogin":"app-user"}"#,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn connects_on_its_own_with_read_only_when_gh_is_logged_in_and_no_account_exists() {
    let gh = FakeGh::new();
    gh.log_in();
    let api = FakeGithub::start().await;
    let repos = repositories().await;
    let (_keep, prefs) = prefs_dir();
    store::reset_test_store_writes();

    let account = detect_and_connect(&repos, &gh.env(), &prefs, ConnectMode::Automatic)
        .await
        .unwrap()
        .expect("connected via gh");

    assert_eq!(account.account_id, USER_ID);
    assert_eq!(account.provider, ConnectorProvider::Github);
    assert_eq!(account.email, "octo");
    assert!(account.via_gh);
    assert_eq!(account.status, ConnectorAccountStatus::Connected);
    assert_eq!(account.scopes, vec![policy::GITHUB_READ.to_string()]);
    assert_eq!(api.paths(), vec!["/user".to_string()]);

    let record = repos.get_connector_account(USER_ID).await.unwrap().unwrap();
    assert!(is_gh_account(&record.metadata));
    assert!(!record.metadata.contains(TOKEN), "no token in the metadata");
    assert_eq!(
        store::test_store_writes(),
        0,
        "nothing goes to the token store"
    );
}

#[tokio::test]
async fn does_not_connect_when_a_github_app_account_exists() {
    let gh = FakeGh::new();
    gh.log_in();
    let _api = FakeGithub::start().await;
    let repos = repositories().await;
    app_account(&repos).await;
    let (_keep, prefs) = prefs_dir();

    let outcome = detect_and_connect(&repos, &gh.env(), &prefs, ConnectMode::Automatic)
        .await
        .unwrap();

    assert!(outcome.is_none());
    assert_eq!(repos.list_connector_accounts().await.unwrap().len(), 1);
    assert_eq!(gh.calls("auth token"), 0);
}

#[tokio::test]
async fn does_nothing_without_gh_or_when_gh_is_logged_out() {
    let _api = FakeGithub::start().await;
    let repos = repositories().await;
    let (_keep, prefs) = prefs_dir();
    let empty = tempfile::tempdir().unwrap();

    let missing = detect_and_connect(
        &repos,
        &login_env_with_path(empty.path()),
        &prefs,
        ConnectMode::Automatic,
    )
    .await
    .unwrap();
    assert!(missing.is_none());

    let gh = FakeGh::new();
    let logged_out = detect_and_connect(&repos, &gh.env(), &prefs, ConnectMode::Automatic)
        .await
        .unwrap();
    assert!(logged_out.is_none());
    assert!(repos.list_connector_accounts().await.unwrap().is_empty());
}

#[tokio::test]
async fn reads_the_token_from_gh_on_every_call_and_never_stores_it() {
    let gh = FakeGh::new();
    gh.log_in();
    let _api = FakeGithub::start().await;
    let repos = repositories().await;
    let (_keep, prefs) = prefs_dir();
    detect_and_connect(&repos, &gh.env(), &prefs, ConnectMode::Automatic)
        .await
        .unwrap()
        .unwrap();
    store::reset_test_store_writes();
    let before = gh.calls("auth token");

    let first = access_token(&repos, &gh.env(), USER_ID).await.unwrap();
    let second = access_token(&repos, &gh.env(), USER_ID).await.unwrap();

    assert_eq!(first, TOKEN);
    assert_eq!(second, TOKEN);
    assert_eq!(gh.calls("auth token") - before, 2);
    assert_eq!(store::test_store_writes(), 0);
}

#[tokio::test]
async fn gh_logout_turns_the_account_into_reconnect_required() {
    let gh = FakeGh::new();
    gh.log_in();
    let _api = FakeGithub::start().await;
    let repos = repositories().await;
    let (_keep, prefs) = prefs_dir();
    detect_and_connect(&repos, &gh.env(), &prefs, ConnectMode::Automatic)
        .await
        .unwrap()
        .unwrap();

    gh.log_out();
    let error = access_token(&repos, &gh.env(), USER_ID).await.unwrap_err();

    assert_eq!(error.code, "connector_reconnect_required");
    assert!(!error.message.contains(TOKEN));
    let record = repos.get_connector_account(USER_ID).await.unwrap().unwrap();
    assert_eq!(
        record.status,
        ConnectorAccountStatus::ReconnectRequired.as_str()
    );

    // Logging back in heals it on the next detection.
    gh.log_in();
    detect_and_connect(&repos, &gh.env(), &prefs, ConnectMode::Automatic)
        .await
        .unwrap();
    let record = repos.get_connector_account(USER_ID).await.unwrap().unwrap();
    assert_eq!(record.status, ConnectorAccountStatus::Connected.as_str());
}

#[tokio::test]
async fn disconnecting_stops_automatic_reconnection_until_an_explicit_connect() {
    let gh = FakeGh::new();
    gh.log_in();
    let _api = FakeGithub::start().await;
    let repos = repositories().await;
    let (_keep, prefs) = prefs_dir();
    assert!(auto_connect_enabled(&prefs));
    detect_and_connect(&repos, &gh.env(), &prefs, ConnectMode::Automatic)
        .await
        .unwrap()
        .unwrap();

    disconnect(&repos, &prefs, USER_ID).await.unwrap();
    assert!(!auto_connect_enabled(&prefs));
    assert!(repos
        .get_connector_account(USER_ID)
        .await
        .unwrap()
        .is_none());

    let automatic = detect_and_connect(&repos, &gh.env(), &prefs, ConnectMode::Automatic)
        .await
        .unwrap();
    assert!(automatic.is_none());

    let explicit = detect_and_connect(&repos, &gh.env(), &prefs, ConnectMode::Explicit)
        .await
        .unwrap();
    assert!(explicit.is_some_and(|account| account.via_gh));
    assert!(auto_connect_enabled(&prefs));
}

#[tokio::test]
async fn explicit_connect_explains_a_missing_or_logged_out_gh() {
    let _api = FakeGithub::start().await;
    let repos = repositories().await;
    let (_keep, prefs) = prefs_dir();
    let empty = tempfile::tempdir().unwrap();

    let missing = detect_and_connect(
        &repos,
        &login_env_with_path(empty.path()),
        &prefs,
        ConnectMode::Explicit,
    )
    .await
    .unwrap_err();
    assert_eq!(missing.code, "connector_github_cli_missing");

    let gh = FakeGh::new();
    let logged_out = detect_and_connect(&repos, &gh.env(), &prefs, ConnectMode::Explicit)
        .await
        .unwrap_err();
    assert_eq!(logged_out.code, "connector_github_cli_logged_out");
}

#[tokio::test]
async fn write_needs_the_write_bundle_like_an_app_account() {
    let gh = FakeGh::new();
    gh.log_in();
    let _api = FakeGithub::start().await;
    let repos = repositories().await;
    let (_keep, prefs) = prefs_dir();
    detect_and_connect(&repos, &gh.env(), &prefs, ConnectMode::Automatic)
        .await
        .unwrap()
        .unwrap();

    let accounts = vec![account_from_record(
        repos.get_connector_account(USER_ID).await.unwrap().unwrap(),
    )];
    let refused = crate::connectors::check_account_grant(
        &accounts,
        USER_ID,
        ConnectorProvider::Github,
        policy::GITHUB_WRITE,
        false,
    )
    .unwrap_err();
    assert_eq!(refused.code, "connector_scope_missing");

    let write = policy::ScopeBundle::from_name("github_write").unwrap();
    let granted = grant_bundles(&repos, USER_ID, &[write]).await.unwrap();
    assert!(granted.via_gh);
    assert!(granted.scopes.contains(&policy::GITHUB_WRITE.to_string()));
    let accounts = vec![account_from_record(
        repos.get_connector_account(USER_ID).await.unwrap().unwrap(),
    )];
    crate::connectors::check_account_grant(
        &accounts,
        USER_ID,
        ConnectorProvider::Github,
        policy::GITHUB_WRITE,
        false,
    )
    .unwrap();
}

#[tokio::test]
async fn lists_repositories_of_a_gh_account_from_the_user_repos_endpoint() {
    let _api = FakeGithub::start().await;

    let list = github::list_user_repositories(TOKEN).await.unwrap();

    assert_eq!(list.repositories.len(), 1);
    assert_eq!(list.repositories[0].full_name, "octo/hello");
    assert!(!list.truncated);
    assert!(_api
        .paths()
        .iter()
        .all(|path| path.starts_with("/user/repos")));
}

#[tokio::test]
async fn refuses_a_gh_token_that_belongs_to_another_github_user() {
    let gh = FakeGh::new();
    gh.log_in();
    let _api = FakeGithub::start().await;
    let repos = repositories().await;
    let (_keep, prefs) = prefs_dir();
    detect_and_connect(&repos, &gh.env(), &prefs, ConnectMode::Automatic)
        .await
        .unwrap()
        .unwrap();

    // gh auth switch to another account.
    gh.log_in_with(OTHER_TOKEN);
    let error = access_token(&repos, &gh.env(), USER_ID).await.unwrap_err();

    assert_eq!(error.code, "connector_account_mismatch");
    assert!(!error.message.contains(OTHER_TOKEN));
    let record = repos.get_connector_account(USER_ID).await.unwrap().unwrap();
    assert_eq!(
        record.status,
        ConnectorAccountStatus::ReconnectRequired.as_str()
    );
}

#[tokio::test]
async fn confirms_the_token_owner_at_most_once_per_minute() {
    let gh = FakeGh::new();
    gh.log_in();
    let api = FakeGithub::start().await;
    let repos = repositories().await;
    let (_keep, prefs) = prefs_dir();
    detect_and_connect(&repos, &gh.env(), &prefs, ConnectMode::Automatic)
        .await
        .unwrap()
        .unwrap();
    let users = |api: &FakeGithub| api.paths().iter().filter(|path| *path == "/user").count();
    let before = users(&api);

    for _ in 0..3 {
        assert_eq!(
            access_token(&repos, &gh.env(), USER_ID).await.unwrap(),
            TOKEN
        );
    }

    assert!(users(&api) - before <= 1, "{:?}", api.paths());
}

#[tokio::test]
async fn explicit_reconnect_with_another_gh_user_reports_the_mismatch() {
    let gh = FakeGh::new();
    gh.log_in();
    let _api = FakeGithub::start().await;
    let repos = repositories().await;
    let (_keep, prefs) = prefs_dir();
    detect_and_connect(&repos, &gh.env(), &prefs, ConnectMode::Automatic)
        .await
        .unwrap()
        .unwrap();
    gh.log_out();
    let _ = access_token(&repos, &gh.env(), USER_ID).await.unwrap_err();
    gh.log_in_with(OTHER_TOKEN);

    let error = detect_and_connect(&repos, &gh.env(), &prefs, ConnectMode::Explicit)
        .await
        .unwrap_err();
    assert_eq!(error.code, "connector_account_mismatch");
    assert!(
        error.message.contains("gh auth switch"),
        "{}",
        error.message
    );

    // The automatic detection stays silent and changes nothing.
    let automatic = detect_and_connect(&repos, &gh.env(), &prefs, ConnectMode::Automatic)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(automatic.account_id, USER_ID);
    assert_eq!(automatic.status, ConnectorAccountStatus::ReconnectRequired);
    assert_eq!(repos.list_connector_accounts().await.unwrap().len(), 1);
}

// U2: the slow gh/HTTP probe runs first; a competing change lands before the
// commit, which must decide on the accounts as they are then.

#[tokio::test]
async fn a_pending_detection_does_not_recreate_a_disconnected_account() {
    let gh = FakeGh::new();
    gh.log_in();
    let _api = FakeGithub::start().await;
    let repos = repositories().await;
    let (_keep, prefs) = prefs_dir();
    detect_and_connect(&repos, &gh.env(), &prefs, ConnectMode::Automatic)
        .await
        .unwrap()
        .unwrap();

    let pending = probe_gh(&gh.env()).await;
    disconnect(&repos, &prefs, USER_ID).await.unwrap();
    let outcome = commit(&repos, &prefs, ConnectMode::Automatic, pending)
        .await
        .unwrap();

    assert!(outcome.is_none());
    assert!(repos.list_connector_accounts().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_pending_detection_does_not_add_a_second_account_after_an_app_connect() {
    let gh = FakeGh::new();
    gh.log_in();
    let _api = FakeGithub::start().await;
    let repos = repositories().await;
    let (_keep, prefs) = prefs_dir();

    let pending = probe_gh(&gh.env()).await;
    app_account(&repos).await;
    let outcome = commit(&repos, &prefs, ConnectMode::Automatic, pending)
        .await
        .unwrap();

    assert!(outcome.is_none());
    let accounts = repos.list_connector_accounts().await.unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].account_id, "999");
}

#[tokio::test]
async fn a_pending_detection_does_not_downgrade_a_write_grant() {
    let gh = FakeGh::new();
    gh.log_in();
    let _api = FakeGithub::start().await;
    let repos = repositories().await;
    let (_keep, prefs) = prefs_dir();

    let pending = probe_gh(&gh.env()).await;
    detect_and_connect(&repos, &gh.env(), &prefs, ConnectMode::Automatic)
        .await
        .unwrap()
        .unwrap();
    let write = policy::ScopeBundle::from_name("github_write").unwrap();
    grant_bundles(&repos, USER_ID, &[write]).await.unwrap();
    commit(&repos, &prefs, ConnectMode::Automatic, pending)
        .await
        .unwrap();

    let record = repos.get_connector_account(USER_ID).await.unwrap().unwrap();
    assert!(
        record.scopes.contains(&policy::GITHUB_WRITE.to_string()),
        "{:?}",
        record.scopes
    );
}

#[tokio::test]
async fn the_commit_waits_for_the_github_account_lifecycle_lock() {
    let gh = FakeGh::new();
    gh.log_in();
    let _api = FakeGithub::start().await;
    let repos = repositories().await;
    let (keep, prefs) = prefs_dir();
    let pending = probe_gh(&gh.env()).await;

    let guard = github_accounts_lock().lock().await;
    let committing = {
        let repos = repos.clone();
        tokio::spawn(async move {
            let _keep = keep;
            commit(&repos, &prefs, ConnectMode::Automatic, pending).await
        })
    };
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert!(!committing.is_finished());
    assert!(repos.list_connector_accounts().await.unwrap().is_empty());

    drop(guard);
    let account = committing.await.unwrap().unwrap().unwrap();
    assert_eq!(account.account_id, USER_ID);
}
