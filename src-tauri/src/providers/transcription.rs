//! Resolves transcription before reading credentials. There is no provider fallback.
use super::{ProviderModelSettings, DEFAULT_TRANSCRIPTION_MODEL};
use crate::domain::types::AppError;
use std::path::PathBuf;

pub const LOCAL_MODEL: &str = "parakeet-ultra";
#[derive(Clone)]
pub enum TranscriptionBackend {
    Groq { key: String, model: String },
    Local { model_dir: PathBuf },
}
impl std::fmt::Debug for TranscriptionBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Groq { model, .. } => f
                .debug_struct("Groq")
                .field("model", model)
                .finish_non_exhaustive(),
            Self::Local { model_dir } => f
                .debug_struct("Local")
                .field("model_dir", model_dir)
                .finish(),
        }
    }
}
pub fn effective_transcription_backend(
    settings: &ProviderModelSettings,
    model_dir: Option<PathBuf>,
    supported: bool,
    read_key: impl FnOnce() -> Result<Option<String>, AppError>,
) -> Result<TranscriptionBackend, AppError> {
    if settings.transcription_provider == super::PROVIDER_LOCAL && supported {
        return model_dir
            .map(|model_dir| TranscriptionBackend::Local { model_dir })
            .ok_or_else(|| {
                AppError::new(
                    "local_model_not_installed",
                    "Install the local model in Settings > Models.",
                )
            });
    }
    let key = read_key()?;
    Ok(TranscriptionBackend::Groq {
        key: super::groq::require_groq_key(key.as_deref())?.into(),
        model: if settings.transcription_provider == super::PROVIDER_LOCAL {
            DEFAULT_TRANSCRIPTION_MODEL.into()
        } else {
            settings.transcription_model.clone()
        },
    })
}

pub fn ready_directory() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        crate::transcription_model::ready_directory()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}
#[cfg(test)]
tokio::task_local! { pub(crate) static TEST_BACKEND: TranscriptionBackend; }
pub fn current_backend() -> Result<TranscriptionBackend, AppError> {
    #[cfg(test)]
    if let Ok(backend) = TEST_BACKEND.try_with(Clone::clone) {
        return Ok(backend);
    }
    effective_transcription_backend(
        &super::effective_current_settings(),
        ready_directory(),
        super::local_transcription_supported(),
        || super::provider_api_key(super::BuiltinProvider::Groq),
    )
}
pub fn require_local_ready() -> Result<(), AppError> {
    if !super::local_transcription_supported() {
        return Err(AppError::new(
            "local_transcription_unsupported",
            "Local transcription requires macOS 14 or later on Apple Silicon.",
        ));
    }
    ready_directory().map(|_| ()).ok_or_else(|| {
        AppError::new(
            "local_model_not_installed",
            "Install the local model in Settings > Models.",
        )
    })
}

pub fn warm_dictation_route_if_idle() {
    if super::configured_transcription_provider() == super::PROVIDER_LOCAL {
        super::local_transcriber::selection_changed();
    } else {
        super::groq::warm_dictation_route_if_idle();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_transcription_ready_never_reads_groq_credentials() {
        let mut settings = super::super::default_settings_for_tests();
        settings.transcription_provider = "local".into();
        let backend = effective_transcription_backend(
            &settings,
            Some(PathBuf::from("/fixture/model")),
            true,
            || panic!("Local read a Groq key"),
        )
        .unwrap();
        assert!(matches!(backend, TranscriptionBackend::Local { .. }));
    }
    #[test]
    fn local_transcription_missing_model_fails_without_groq_fallback() {
        let mut settings = super::super::default_settings_for_tests();
        settings.transcription_provider = "local".into();
        let error = effective_transcription_backend(&settings, None, true, || {
            panic!("Missing Local model read Groq credentials")
        })
        .unwrap_err();
        assert_eq!(error.code, "local_model_not_installed");
    }
    #[test]
    fn local_transcription_unsupported_platform_uses_a_groq_model() {
        let mut settings = super::super::default_settings_for_tests();
        settings.transcription_provider = "local".into();
        settings.transcription_model = LOCAL_MODEL.into();
        let backend = effective_transcription_backend(
            &settings,
            Some(PathBuf::from("/fixture/model")),
            false,
            || Ok(Some("fixture-key".into())),
        )
        .unwrap();
        assert!(
            matches!(backend, TranscriptionBackend::Groq { model, .. } if model == DEFAULT_TRANSCRIPTION_MODEL)
        );
    }
    #[test]
    fn local_transcription_resolver_preserves_groq_model_and_key_errors() {
        let mut settings = super::super::default_settings_for_tests();
        settings.transcription_model = "whisper-large-v3".into();
        let backend = effective_transcription_backend(&settings, None, true, || {
            Ok(Some("fixture-key".into()))
        })
        .unwrap();
        assert!(
            matches!(backend, TranscriptionBackend::Groq { model, .. } if model == "whisper-large-v3")
        );
        let error = effective_transcription_backend(&settings, None, true, || {
            Err(AppError::new("key_unreadable", "fixture"))
        })
        .unwrap_err();
        assert_eq!(error.code, "key_unreadable");
    }
}
