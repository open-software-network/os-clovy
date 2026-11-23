//! Interface language for the native chrome (app menu and menu bar tray).
//!
//! The webview owns the preference (`src/i18n/locale.ts`, localStorage) and
//! emits `clovy://interface-locale` with the chosen BCP 47 tag on startup and
//! on every change. Rust mirrors it so the menus it builds read in the same
//! language (unknown tags fall back to English, matching the frontend), and
//! saves it in the config dir (`SAVED_LOCALE_FILE`): activity text generated
//! at startup, before the webview loads, needs it, and waits while it is
//! unknown. This is independent of the dictation/transcription language.

use std::sync::atomic::{AtomicU8, Ordering};

pub const INTERFACE_LOCALE_EVENT: &str = "clovy://interface-locale";
/// Sent once the listener exists: the main window may have broadcast before
/// it (Tauri builds the configured window before `setup`), so it answers.
pub const INTERFACE_LOCALE_REQUEST_EVENT: &str = "clovy://interface-locale-request";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiLocale {
    En,
    PtBr,
}

static CURRENT: LocaleState = LocaleState::new();

impl UiLocale {
    /// Parse an event payload: a JSON string ("\"pt-BR\"") or a bare tag.
    pub fn from_payload(payload: &str) -> Self {
        let tag = serde_json::from_str::<String>(payload).unwrap_or_else(|_| payload.to_string());
        Self::from_tag(&tag)
    }

    pub fn from_tag(tag: &str) -> Self {
        let tag = tag.trim().replace('_', "-").to_ascii_lowercase();
        if tag == "pt" || tag.starts_with("pt-") {
            UiLocale::PtBr
        } else {
            UiLocale::En
        }
    }

    /// The BCP 47 tag the webview uses.
    pub fn tag(self) -> &'static str {
        match self {
            UiLocale::En => "en",
            UiLocale::PtBr => "pt-BR",
        }
    }

    fn as_u8(self) -> u8 {
        match self {
            UiLocale::En => 1,
            UiLocale::PtBr => 2,
        }
    }

    fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(UiLocale::En),
            2 => Some(UiLocale::PtBr),
            _ => None,
        }
    }
}

/// The interface language for the native chrome: English until known.
pub fn current() -> UiLocale {
    CURRENT.current()
}

/// The interface language, once the webview said it or the saved file did.
/// Generated activity text waits for it.
pub fn known() -> Option<UiLocale> {
    CURRENT.known()
}

/// Restores the language saved in `dir` at startup, before anything is
/// generated. Returns it when there was one.
pub fn restore(dir: &std::path::Path) -> Option<UiLocale> {
    let locale = read_saved(dir)?;
    CURRENT.set(locale);
    Some(locale)
}

/// Asks the main window for its language (`ask`) until it is `known`, waiting
/// longer between asks up to 30 s. One ask at startup is not enough: on
/// Windows the WebView loads alongside a long `setup`, and neither the
/// window's broadcast nor Rust's first request has a guaranteed order.
pub fn ask_until_known(
    ask: impl Fn(),
    known: impl Fn() -> bool,
    wait: impl Fn(std::time::Duration),
) {
    let mut delay = std::time::Duration::from_millis(500);
    while !known() {
        ask();
        wait(delay);
        delay = (delay * 2).min(std::time::Duration::from_secs(30));
    }
}

/// File in the app config dir holding the last interface language, so text
/// generated before the webview loads already uses it.
pub const SAVED_LOCALE_FILE: &str = "interface-locale.json";

/// The interface language once known (event or saved file), else nothing.
#[derive(Debug, Default)]
pub struct LocaleState(AtomicU8);

impl LocaleState {
    pub const fn new() -> Self {
        Self(AtomicU8::new(0))
    }

    pub fn known(&self) -> Option<UiLocale> {
        UiLocale::from_u8(self.0.load(Ordering::SeqCst))
    }

    pub fn current(&self) -> UiLocale {
        self.known().unwrap_or(UiLocale::En)
    }

    /// Stores the locale; returns true when it changed.
    pub fn set(&self, locale: UiLocale) -> bool {
        self.0.swap(locale.as_u8(), Ordering::SeqCst) != locale.as_u8()
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
struct SavedLocale {
    locale: String,
}

/// The saved interface language in `dir`, if any is readable.
pub fn read_saved(dir: &std::path::Path) -> Option<UiLocale> {
    let text = std::fs::read_to_string(dir.join(SAVED_LOCALE_FILE)).ok()?;
    let saved: SavedLocale = serde_json::from_str(&text).ok()?;
    Some(UiLocale::from_tag(&saved.locale))
}

/// Saves the interface language in `dir`.
pub fn write_saved(dir: &std::path::Path, locale: UiLocale) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let saved = SavedLocale {
        locale: locale.tag().to_string(),
    };
    std::fs::write(
        dir.join(SAVED_LOCALE_FILE),
        serde_json::to_vec(&saved).map_err(std::io::Error::other)?,
    )
}

/// Store the locale; returns true when it changed.
pub fn set_current(locale: UiLocale) -> bool {
    CURRENT.set(locale)
}

/// Native menu copy. English strings are byte-identical to the pre-i18n
/// literals; Portuguese follows the repo copy rules (sentence case, plain
/// hyphens).
#[derive(Clone, Copy, Debug)]
pub struct MenuStrings {
    locale: UiLocale,
}

pub fn strings(locale: UiLocale) -> MenuStrings {
    MenuStrings { locale }
}

impl MenuStrings {
    fn pick(&self, en: &'static str, pt: &'static str) -> &'static str {
        match self.locale {
            UiLocale::En => en,
            UiLocale::PtBr => pt,
        }
    }

    pub fn settings(&self) -> &'static str {
        self.pick("Settings...", "Configurações...")
    }
    pub fn file(&self) -> &'static str {
        self.pick("File", "Arquivo")
    }
    pub fn edit(&self) -> &'static str {
        self.pick("Edit", "Editar")
    }
    pub fn view(&self) -> &'static str {
        self.pick("View", "Visualizar")
    }
    pub fn window(&self) -> &'static str {
        self.pick("Window", "Janela")
    }
    pub fn help(&self) -> &'static str {
        self.pick("Help", "Ajuda")
    }
    pub fn close_tab(&self) -> &'static str {
        self.pick("Close tab", "Fechar aba")
    }
    pub fn close_window(&self) -> &'static str {
        self.pick("Close window", "Fechar janela")
    }

    /// Text for AppKit's predefined items. `None` keeps the platform default
    /// (English), which is exactly the pre-i18n behavior.
    pub fn predefined(&self, item: PredefinedLabel) -> Option<&'static str> {
        if self.locale == UiLocale::En {
            return None;
        }
        Some(match item {
            PredefinedLabel::Services => "Serviços",
            PredefinedLabel::Hide => "Ocultar Clovy",
            PredefinedLabel::HideOthers => "Ocultar outros",
            PredefinedLabel::ShowAll => "Mostrar tudo",
            PredefinedLabel::Quit => "Encerrar Clovy",
            PredefinedLabel::Undo => "Desfazer",
            PredefinedLabel::Redo => "Refazer",
            PredefinedLabel::Cut => "Recortar",
            PredefinedLabel::Copy => "Copiar",
            PredefinedLabel::Paste => "Colar",
            PredefinedLabel::SelectAll => "Selecionar tudo",
            PredefinedLabel::Fullscreen => "Tela cheia",
            PredefinedLabel::Minimize => "Minimizar",
            PredefinedLabel::Maximize => "Zoom",
        })
    }

    pub fn predefined_about(&self) -> Option<&'static str> {
        match self.locale {
            UiLocale::En => None,
            UiLocale::PtBr => Some("Sobre o Clovy"),
        }
    }

    // Menu bar tray.
    pub fn open_clovy(&self) -> &'static str {
        self.pick("Open Clovy", "Abrir o Clovy")
    }
    pub fn quit_clovy(&self) -> &'static str {
        self.pick("Quit Clovy", "Encerrar Clovy")
    }
    pub fn activity(&self, recording: bool, dictating: bool) -> &'static str {
        match (recording, dictating) {
            (true, true) => self.pick("Recording and dictating", "Gravando e ditando"),
            (true, false) => self.pick("Recording", "Gravando"),
            (false, true) => self.pick("Dictating", "Ditando"),
            (false, false) => "",
        }
    }

    pub fn activity_status(&self, state: &crate::activity::schedule::CaptureState) -> String {
        use crate::activity::schedule::CaptureState;
        use crate::activity::store::PauseReason;
        let detail = match state {
            CaptureState::Off => self.pick("off", "desligada"),
            CaptureState::Active => self.pick("on", "ativa"),
            CaptureState::Paused { reason } => match reason {
                PauseReason::Manual => self.pick("paused", "pausada"),
                PauseReason::WorkHours => {
                    self.pick("paused, outside work hours", "pausada, fora do horário")
                }
                PauseReason::LowDisk => {
                    self.pick("paused, low disk space", "pausada, pouco espaço em disco")
                }
                PauseReason::ProtectedVideo => {
                    self.pick("paused, protected video", "pausada, vídeo protegido")
                }
            },
            CaptureState::NeedsPermissions { .. } => {
                self.pick("needs permissions", "precisa de permissões")
            }
            CaptureState::KeyMissing => self.pick("database key missing", "chave do banco ausente"),
            CaptureState::Error { .. } => self.pick("unavailable", "indisponível"),
        };
        match self.locale {
            UiLocale::En => format!("Activity capture: {detail}"),
            UiLocale::PtBr => format!("Captura de atividade: {detail}"),
        }
    }
    pub fn pause_capture(&self) -> &'static str {
        self.pick("Pause capture", "Pausar captura")
    }
    pub fn resume_capture(&self) -> &'static str {
        self.pick("Resume capture", "Retomar captura")
    }
}

#[derive(Clone, Copy, Debug)]
pub enum PredefinedLabel {
    Services,
    Hide,
    HideOthers,
    ShowAll,
    Quit,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Fullscreen,
    Minimize,
    Maximize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_payloads_and_falls_back_to_english() {
        assert_eq!(UiLocale::from_payload("\"pt-BR\""), UiLocale::PtBr);
        assert_eq!(UiLocale::from_payload("pt_br"), UiLocale::PtBr);
        assert_eq!(UiLocale::from_payload("\"en\""), UiLocale::En);
        assert_eq!(UiLocale::from_payload("\"fr\""), UiLocale::En);
        assert_eq!(UiLocale::from_payload("not json"), UiLocale::En);
        assert_eq!(UiLocale::from_payload(""), UiLocale::En);
    }

    #[test]
    fn rust_keeps_asking_for_the_language_until_it_is_known() {
        use std::cell::{Cell, RefCell};
        use std::time::Duration;
        let asks = Cell::new(0);
        let waits = RefCell::new(Vec::new());
        ask_until_known(
            || asks.set(asks.get() + 1),
            || asks.get() >= 4,
            |wait| waits.borrow_mut().push(wait),
        );
        assert_eq!(asks.get(), 4);
        let waits = waits.into_inner();
        assert_eq!(waits.len(), 4);
        assert!(waits.windows(2).all(|pair| pair[0] <= pair[1]), "{waits:?}");
        assert!(waits.iter().all(|wait| *wait <= Duration::from_secs(30)));

        let asked = Cell::new(false);
        ask_until_known(|| asked.set(true), || true, |_| {});
        assert!(!asked.get(), "a known language is not asked for");
    }

    #[test]
    fn the_language_is_unknown_until_set() {
        let state = LocaleState::new();
        assert_eq!(state.known(), None);
        assert_eq!(state.current(), UiLocale::En);
        assert!(state.set(UiLocale::PtBr));
        assert_eq!(state.known(), Some(UiLocale::PtBr));
        assert_eq!(state.current(), UiLocale::PtBr);
        assert!(!state.set(UiLocale::PtBr));
        assert!(state.set(UiLocale::En));
        assert_eq!(state.known(), Some(UiLocale::En));
    }

    #[test]
    fn the_saved_language_survives_a_restart_and_garbage_is_unknown() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_saved(dir.path()), None);
        write_saved(dir.path(), UiLocale::PtBr).unwrap();
        assert_eq!(read_saved(dir.path()), Some(UiLocale::PtBr));
        write_saved(dir.path(), UiLocale::En).unwrap();
        assert_eq!(read_saved(dir.path()), Some(UiLocale::En));
        std::fs::write(dir.path().join(SAVED_LOCALE_FILE), "not json").unwrap();
        assert_eq!(read_saved(dir.path()), None);
    }

    #[test]
    fn english_keeps_platform_defaults_for_predefined_items() {
        let en = strings(UiLocale::En);
        assert_eq!(en.predefined(PredefinedLabel::Quit), None);
        assert_eq!(en.settings(), "Settings...");
        let pt = strings(UiLocale::PtBr);
        assert_eq!(pt.predefined(PredefinedLabel::Quit), Some("Encerrar Clovy"));
        assert_eq!(pt.settings(), "Configurações...");
    }
}
