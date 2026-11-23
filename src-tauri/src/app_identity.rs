//! The app's own identity (ADR-0068): the identifier and the names derived
//! from it, so no other module spells them.

/// The published identifier. Its data directory belongs to the installed app.
pub const RELEASE_APP_IDENTIFIER: &str = "dev.guilhermexp.clovy";

/// The identifier this build was compiled with (`build.rs`): the release
/// identifier, or a development identity such as `CLOVY_DEV_APP_IDENTIFIER`.
pub const APP_IDENTIFIER: &str = env!("CLOVY_APP_IDENTIFIER");

/// The app bundle's main executable (`Contents/MacOS/clovy`, `clovy.exe`).
pub const MAIN_EXECUTABLE: &str = "clovy";

/// A published helper's bundle identifier, e.g. `computer-use-driver`.
pub fn release_helper_identifier(helper: &str) -> String {
    format!("{RELEASE_APP_IDENTIFIER}.{helper}")
}

/// The credential service prefix: the identifier, with `-dev` in debug builds
/// so a development binary never reads or rotates release credentials.
pub fn credential_service_prefix_for(identifier: &str, debug_assertions: bool) -> String {
    if debug_assertions {
        format!("{identifier}-dev")
    } else {
        identifier.to_string()
    }
}

/// `<prefix>.<name>` for one credential service, e.g. `activity-db`.
pub fn credential_service_for(identifier: &str, debug_assertions: bool, name: &str) -> String {
    format!(
        "{}.{name}",
        credential_service_prefix_for(identifier, debug_assertions)
    )
}

/// The credential service this build reads and writes for `name`.
pub fn credential_service(name: &str) -> String {
    credential_service_for(APP_IDENTIFIER, cfg!(debug_assertions), name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_services_separate_release_and_development() {
        assert_eq!(
            credential_service_for(RELEASE_APP_IDENTIFIER, false, "activity-db"),
            "dev.guilhermexp.clovy.activity-db"
        );
        assert_eq!(
            credential_service_for(RELEASE_APP_IDENTIFIER, true, "activity-db"),
            "dev.guilhermexp.clovy-dev.activity-db"
        );
    }
}
