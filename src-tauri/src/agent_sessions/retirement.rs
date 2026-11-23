//! Cleanup of one-shot secrets created by the retired native chat engine.
//! The SQL queue contains opaque references only, never credential values.
use crate::domain::types::AppError;
use sqlx::row::Row;
use sqlx_sqlite::SqlitePool;
use std::sync::Arc;

fn services(identifier: &str, debug: bool) -> Vec<String> {
    let mut services = vec![crate::app_identity::credential_service_for(
        identifier,
        debug,
        "agent-secrets",
    )];
    // A proof identifier must never reach the owner's credentials or a legacy service.
    if identifier == crate::app_identity::RELEASE_APP_IDENTIFIER {
        for legacy in ["co.opensoftware.clovy", "co.opensoftware.june"] {
            if debug {
                services.push(format!("{legacy}.dev.agent-secrets"));
            } else {
                services.push(format!("{legacy}.agent-secrets"));
            }
        }
    }
    services
}

pub(crate) async fn cleanup_secrets(pool: &SqlitePool) -> Result<(), AppError> {
    cleanup_secrets_with(
        pool,
        services(crate::app_identity::APP_IDENTIFIER, cfg!(debug_assertions)),
        Arc::new(|service: &str, secret_ref: &str| {
            crate::credentials::delete_password(service, secret_ref).is_ok()
        }),
    )
    .await
}

async fn cleanup_secrets_with(
    pool: &SqlitePool,
    services: Vec<String>,
    delete: Arc<impl Fn(&str, &str) -> bool + Send + Sync + 'static>,
) -> Result<(), AppError> {
    let refs = sqlx::query::query(
        "SELECT secret_ref FROM agent_secret_retirements WHERE completed_at IS NULL",
    )
    .fetch_all(pool)
    .await?;
    for row in refs {
        let secret_ref: String = row.get("secret_ref");
        let owned_ref = secret_ref.clone();
        let services = services.clone();
        let delete = delete.clone();
        let removed = tokio::task::spawn_blocking(move || {
            services.iter().fold(true, |removed, service| {
                // A failed current service must not starve legacy services.
                let service_removed = delete(service, &owned_ref);
                removed && service_removed
            })
        })
        .await
        .unwrap_or(false);
        if removed {
            sqlx::query::query(
                "UPDATE agent_secret_retirements SET completed_at = ? WHERE secret_ref = ?",
            )
            .bind(crate::db::repositories::timestamp())
            .bind(secret_ref)
            .execute(pool)
            .await?;
        } else {
            tracing::warn!("Retired agent secret cleanup will retry on the next launch");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cleanup_attempts_all_services_and_retries_only_incomplete_references() {
        let pool = sqlx_sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query::query("CREATE TABLE agent_secret_retirements (secret_ref TEXT PRIMARY KEY, completed_at TEXT)")
            .execute(&pool).await.unwrap();
        sqlx::query::query(
            "INSERT INTO agent_secret_retirements (secret_ref) VALUES ('retry'), ('done')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = calls.clone();
        cleanup_secrets_with(
            &pool,
            vec!["current".into(), "legacy".into()],
            Arc::new(move |service: &str, reference: &str| {
                recorded
                    .lock()
                    .unwrap()
                    .push((service.to_string(), reference.to_string()));
                !(service == "current" && reference == "retry")
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            calls.lock().unwrap().len(),
            4,
            "A failed service must not prevent the other service from being cleaned"
        );
        let pending: Vec<String> = sqlx::query_scalar::query_scalar(
            "SELECT secret_ref FROM agent_secret_retirements WHERE completed_at IS NULL",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(pending, ["retry"]);
        calls.lock().unwrap().clear();
        let recorded = calls.clone();
        cleanup_secrets_with(
            &pool,
            vec!["current".into(), "legacy".into()],
            Arc::new(move |service: &str, reference: &str| {
                recorded
                    .lock()
                    .unwrap()
                    .push((service.to_string(), reference.to_string()));
                true
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            *calls.lock().unwrap(),
            [
                ("current".into(), "retry".into()),
                ("legacy".into(), "retry".into())
            ]
        );
        cleanup_secrets_with(
            &pool,
            vec!["current".into(), "legacy".into()],
            Arc::new(|_: &str, _: &str| panic!("completed cleanup must not access credentials")),
        )
        .await
        .unwrap();
    }

    #[test]
    fn proof_cleanup_never_opens_owner_or_legacy_services() {
        assert_eq!(
            services("dev.guilhermexp.clovy.testproof", true),
            vec!["dev.guilhermexp.clovy.testproof-dev.agent-secrets"]
        );
        assert!(services(crate::app_identity::RELEASE_APP_IDENTIFIER, true)
            .iter()
            .all(|name| name.contains("-dev.") || name.contains(".dev.")));
        assert!(services(crate::app_identity::RELEASE_APP_IDENTIFIER, false)
            .contains(&"co.opensoftware.clovy.agent-secrets".to_string()));
    }
}
