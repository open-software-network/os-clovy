use serde::{Deserialize, Serialize};

// Deliberately conservative (ADR 0008 addendum, JUN-209): this list only
// gates the safe-mode consent dialog, never what gets generated - the image
// provider's safety filter, requested through `safe_mode`, is the enforcement.
// Keep terms unambiguous; euphemisms are an accepted miss, and broadening the
// list mostly buys false-positive dialogs.
const EXPLICIT_TERMS: &[&str] = &[
    "bdsm",
    "bottomless",
    "erotic",
    "erotica",
    "explicit",
    "fetish",
    "genitals",
    "hentai",
    "naked",
    "nipples",
    "nsfw",
    "nude",
    "nudes",
    "porn",
    "porno",
    "pornographic",
    "sex",
    "sexual",
    "sexy",
    "stripping",
    "striptease",
    "topless",
    "undressed",
    "undressing",
    "xxx",
];

const EXPLICIT_PHRASES: &[&[&str]] = &[
    &["no", "clothes"],
    &["take", "off", "her", "clothes"],
    &["take", "off", "his", "clothes"],
    &["take", "off", "their", "clothes"],
    &["without", "clothes"],
];

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImagePromptScreenRequest {
    pub prompt: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImagePromptScreenResponse {
    pub may_be_explicit: bool,
}

/// True when an image prompt plausibly requests explicit (adult) content, so
/// callers can offer the safe-mode consent dialog before generating.
pub fn may_request_explicit_content(prompt: &str) -> bool {
    let normalized = prompt.to_lowercase();
    let tokens: Vec<&str> = normalized
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect();

    tokens
        .iter()
        .any(|token| EXPLICIT_TERMS.binary_search(token).is_ok())
        || EXPLICIT_PHRASES
            .iter()
            .any(|phrase| contains_token_sequence(&tokens, phrase))
}

#[tauri::command]
pub async fn image_prompt_may_be_explicit(
    request: ImagePromptScreenRequest,
) -> ImagePromptScreenResponse {
    ImagePromptScreenResponse {
        may_be_explicit: may_request_explicit_content(&request.prompt),
    }
}

fn contains_token_sequence(tokens: &[&str], sequence: &[&str]) -> bool {
    !sequence.is_empty()
        && tokens
            .windows(sequence.len())
            .any(|window| window == sequence)
}

#[cfg(test)]
mod tests {
    use super::{
        image_prompt_may_be_explicit, may_request_explicit_content, ImagePromptScreenRequest,
    };

    #[test]
    fn detects_plain_term() {
        assert!(may_request_explicit_content("portrait of a nude figure"));
    }

    #[test]
    fn detects_uppercase_term() {
        assert!(may_request_explicit_content("NSFW illustration"));
    }

    #[test]
    fn detects_term_with_punctuation() {
        assert!(may_request_explicit_content("nude!"));
    }

    #[test]
    fn detects_phrase() {
        assert!(may_request_explicit_content("portrait without clothes"));
    }

    #[test]
    fn ignores_sussex_substring_trap() {
        assert!(!may_request_explicit_content("sunset over Sussex"));
    }

    #[test]
    fn ignores_sextant_substring_trap() {
        assert!(!may_request_explicit_content("sextant on a ship's deck"));
    }

    #[test]
    fn ignores_sexagenarian_substring_trap() {
        assert!(!may_request_explicit_content("a sexagenarian chess player"));
    }

    #[test]
    fn ignores_empty_string() {
        assert!(!may_request_explicit_content(""));
    }

    #[tokio::test]
    async fn wordlist_screen_uses_no_model_for_hits_or_misses() {
        for (prompt, expected) in [
            ("portrait of a nude figure", true),
            ("a red bicycle", false),
            ("", false),
        ] {
            let response = image_prompt_may_be_explicit(ImagePromptScreenRequest {
                prompt: prompt.to_string(),
            })
            .await;
            assert_eq!(response.may_be_explicit, expected);
        }
    }
}
