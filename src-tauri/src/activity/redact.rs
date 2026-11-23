//! Clipboard redaction. Clipboard text is the only content stored with an
//! input event, so secrets are replaced before it reaches the database:
//! private key blocks, `password=`-style assignments, bearer tokens, tokens
//! with well-known provider prefixes, JWTs, card numbers, and long
//! high-entropy strings. The result is truncated.

pub const CLIPBOARD_MAX_CHARS: usize = 1_000;
pub const REDACTED: &str = "[redacted]";
const TRUNCATED: &str = " [truncated]";

/// Assignment keys whose value is a secret (`password=...`, `"token": ...`).
const SECRET_KEYS: &[&str] = &[
    "password",
    "passwd",
    "pwd",
    "pass",
    "secret",
    "token",
    "apikey",
    "api_key",
    "api-key",
    "access_key",
    "accesskey",
    "private_key",
    "client_secret",
    "auth",
    "authorization",
    "credential",
    "credentials",
    "senha",
];

/// Prefixes of provider tokens (OpenAI, Stripe, GitHub, GitLab, Slack, AWS,
/// Google, npm, PyPI, Hugging Face, Anthropic).
const TOKEN_PREFIXES: &[&str] = &[
    "sk-",
    "sk_live_",
    "sk_test_",
    "rk_live_",
    "pk_live_",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "glpat-",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "xoxr-",
    "xoxs-",
    "akia",
    "asia",
    "aiza",
    "ya29.",
    "npm_",
    "pypi-",
    "hf_",
];

pub fn redact_clipboard(text: &str) -> String {
    let without_keys = redact_private_key_blocks(text);
    let mut out = String::with_capacity(without_keys.len());
    let mut redact_next = false;
    // The previous token was a bare secret key (`password`, `API_KEY`), so a
    // following `=` / `:` (spaced or attached to the value) starts a secret.
    let mut after_secret_key = false;
    let mut rest = without_keys.as_str();
    while !rest.is_empty() {
        let split = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let (token, tail) = rest.split_at(split);
        let ws_len = tail
            .char_indices()
            .find(|(_, ch)| !ch.is_whitespace())
            .map_or(tail.len(), |(index, _)| index);
        let (whitespace, tail) = tail.split_at(ws_len);
        if !token.is_empty() {
            let bare = token.trim_matches(|ch: char| matches!(ch, '"' | '\'' | ',' | '{' | '}'));
            let operator_len = assignment_operator_prefix(bare);
            if after_secret_key && operator_len > 0 && operator_len == bare.len() {
                // `password = hunter2`: the value is the next token.
                out.push_str(token);
                redact_next = true;
                after_secret_key = false;
            } else if after_secret_key && operator_len > 0 {
                // `password =hunter2`: operator and value in one token.
                let start = token.find(&bare[..operator_len]).unwrap_or(0) + operator_len;
                out.push_str(&token[..start]);
                out.push_str(REDACTED);
                redact_next = false;
                after_secret_key = false;
            } else {
                let (redacted, follows_key) = redact_token(token);
                if redact_next && !follows_key && looks_like_credential_value(token) {
                    out.push_str(REDACTED);
                } else {
                    out.push_str(&redacted);
                }
                redact_next = follows_key;
                after_secret_key = !follows_key && is_secret_key(bare);
            }
        }
        out.push_str(whitespace);
        rest = tail;
    }
    truncate(out)
}

/// Length of a leading assignment operator (`=`, `:`, `:=`, `=>`), or 0.
fn assignment_operator_prefix(token: &str) -> usize {
    [":=", "=>", "=", ":"]
        .iter()
        .find(|operator| token.starts_with(**operator))
        .map_or(0, |operator| operator.len())
}

fn truncate(text: String) -> String {
    if text.chars().count() <= CLIPBOARD_MAX_CHARS {
        return text;
    }
    let mut truncated: String = text.chars().take(CLIPBOARD_MAX_CHARS).collect();
    truncated.push_str(TRUNCATED);
    truncated
}

fn redact_private_key_blocks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("-----BEGIN") {
        let header_end = rest[start..]
            .find('\n')
            .map_or(rest.len(), |index| start + index);
        if !rest[start..header_end].contains("PRIVATE KEY") {
            out.push_str(&rest[..header_end]);
            rest = &rest[header_end..];
            continue;
        }
        out.push_str(&rest[..start]);
        out.push_str(REDACTED);
        let after = &rest[start..];
        rest = match after.find("-----END") {
            Some(end) => {
                let end_line = after[end + 8..]
                    .find("-----")
                    .map_or(after.len(), |index| end + 8 + index + 5);
                &after[end_line..]
            }
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// Redacts one whitespace-delimited token. Returns whether the token was a
/// bare secret key (`password:`, `Bearer`) whose value is the next token.
fn redact_token(token: &str) -> (String, bool) {
    let lower = token.to_ascii_lowercase();
    let trimmed = lower.trim_matches(|ch: char| matches!(ch, '"' | '\'' | ',' | ';' | '{' | '}'));
    if trimmed == "bearer" || trimmed == "basic" {
        return (token.to_string(), true);
    }
    if let Some(key) = trimmed
        .strip_suffix(':')
        .or_else(|| trimmed.strip_suffix('='))
    {
        if is_secret_key(key.trim_matches(|ch: char| matches!(ch, '"' | '\''))) {
            return (token.to_string(), true);
        }
    }
    // `key=value` / `key:value` pairs, also inside URL query strings.
    if token.contains('=') || token.contains(':') {
        if let Some(redacted) = redact_assignments(token) {
            return (redacted, false);
        }
    }
    let core = token.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != '-');
    if !core.is_empty() && is_secret_value(core) {
        return (token.replacen(core, REDACTED, 1), false);
    }
    (token.to_string(), false)
}

fn redact_assignments(token: &str) -> Option<String> {
    let mut changed = false;
    let mut out = String::with_capacity(token.len());
    let mut segment_start = 0;
    let separators = |ch: char| matches!(ch, '?' | '&' | ';' | ',');
    let bytes: Vec<(usize, char)> = token.char_indices().collect();
    let mut segments = Vec::new();
    for (index, ch) in &bytes {
        if separators(*ch) {
            segments.push((segment_start, *index));
            segments.push((*index, *index + ch.len_utf8()));
            segment_start = *index + ch.len_utf8();
        }
    }
    segments.push((segment_start, token.len()));
    for (start, end) in segments {
        let segment = &token[start..end];
        let split = segment.find(['=', ':']);
        match split {
            Some(at) if at > 0 && at + 1 < segment.len() => {
                let key = segment[..at]
                    .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '{'))
                    .to_ascii_lowercase();
                let value = &segment[at + 1..];
                let secret_value =
                    is_secret_value(value.trim_matches(|ch: char| matches!(ch, '"' | '\'')));
                if (is_secret_key(&key) && !value.starts_with("//")) || secret_value {
                    out.push_str(&segment[..=at]);
                    out.push_str(REDACTED);
                    changed = true;
                } else {
                    out.push_str(segment);
                }
            }
            _ => out.push_str(segment),
        }
    }
    changed.then_some(out)
}

fn is_secret_key(key: &str) -> bool {
    let key = key.trim().trim_start_matches('-').to_ascii_lowercase();
    SECRET_KEYS.iter().any(|known| {
        key == *known || key.ends_with(&format!("_{known}")) || key.ends_with(&format!("-{known}"))
    })
}

/// A value after `password:` or `Bearer` is redacted unless it is clearly
/// not a value (empty or punctuation only).
fn looks_like_credential_value(token: &str) -> bool {
    token.chars().any(|ch| ch.is_alphanumeric())
}

fn is_secret_value(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if TOKEN_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix) && value.len() >= prefix.len() + 12)
    {
        return true;
    }
    is_jwt(value) || is_card_number(value) || is_high_entropy(value)
}

fn is_jwt(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    parts.len() == 3
        && value.starts_with("eyJ")
        && parts.iter().all(|part| {
            part.len() >= 8
                && part
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        })
}

fn is_card_number(value: &str) -> bool {
    let digits: Vec<u32> = value
        .chars()
        .filter(|ch| *ch != '-')
        .map(|ch| ch.to_digit(10))
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default();
    if !(13..=19).contains(&digits.len()) {
        return false;
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(index, digit)| {
            if index % 2 == 1 {
                let doubled = digit * 2;
                if doubled > 9 {
                    doubled - 9
                } else {
                    doubled
                }
            } else {
                *digit
            }
        })
        .sum();
    sum % 10 == 0
}

/// 32+ chars of token alphabet mixing letters and digits, and either both
/// cases or symbols: shaped like a generated key, not like a word or path.
fn is_high_entropy(value: &str) -> bool {
    if value.len() < 32
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '+' | '/' | '='))
    {
        return false;
    }
    let has_digit = value.chars().any(|ch| ch.is_ascii_digit());
    let has_upper = value.chars().any(|ch| ch.is_ascii_uppercase());
    let has_lower = value.chars().any(|ch| ch.is_ascii_lowercase());
    let has_symbol = value
        .chars()
        .any(|ch| matches!(ch, '_' | '-' | '+' | '/' | '='));
    let has_letter = has_upper || has_lower;
    let looks_like_path = value.matches('/').count() > 2;
    has_digit && has_letter && (has_upper && has_lower || has_symbol) && !looks_like_path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_tokens_are_redacted() {
        for secret in [
            "sk-proj-AbCdEf0123456789ghIJklMNopQR",
            "ghp_1234567890abcdefghijABCDEFGHIJ123456",
            "xoxb-123456789012-1234567890123-AbCdEfGhIjKlMnOp",
            "AKIAIOSFODNN7EXAMPLE",
            "AIzaSyA-1234567890abcdefghijklmnopqrstu",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
            "glpat-AbCdEf1234567890xyzw",
        ] {
            let redacted = redact_clipboard(&format!("use {secret} now"));
            assert!(!redacted.contains(secret), "leaked {secret}: {redacted}");
            assert!(redacted.contains(REDACTED));
            assert!(redacted.starts_with("use ") && redacted.ends_with(" now"));
        }
    }

    #[test]
    fn assignments_and_bearer_values_are_redacted() {
        let text =
            "password=hunter2 user=ana\nAuthorization: Bearer abc.def.ghi\n\"api_key\": \"q1w2e3\"";
        let redacted = redact_clipboard(text);
        assert!(!redacted.contains("hunter2"));
        assert!(!redacted.contains("abc.def.ghi"));
        assert!(!redacted.contains("q1w2e3"));
        assert!(redacted.contains("user=ana"));
    }

    #[test]
    fn spaced_assignments_are_redacted() {
        for (text, secret) in [
            ("password = hunter2", "hunter2"),
            ("API_KEY = abc123", "abc123"),
            ("token : xyz789", "xyz789"),
            ("secret =s3cr3t", "s3cr3t"),
            ("\"password\" : \"hunter2\"", "hunter2"),
            ("db_password := letmein", "letmein"),
            (
                "export AWS_SECRET_ACCESS_KEY = wJalrXUtnFEMI",
                "wJalrXUtnFEMI",
            ),
        ] {
            let redacted = redact_clipboard(text);
            assert!(
                !redacted.contains(secret),
                "leaked {secret} from {text:?}: {redacted}"
            );
            assert!(redacted.contains(REDACTED), "{text:?} -> {redacted}");
        }
        // A secret word in prose without an operator is left alone.
        assert_eq!(
            redact_clipboard("the password policy changed"),
            "the password policy changed"
        );
    }

    #[test]
    fn url_query_secrets_are_redacted_but_the_url_stays() {
        let redacted = redact_clipboard("https://api.example.com/v1?token=s3cr3tvalue&page=2");
        assert!(!redacted.contains("s3cr3tvalue"));
        assert!(redacted.contains("api.example.com"));
        assert!(redacted.contains("page=2"));
    }

    #[test]
    fn private_key_blocks_and_card_numbers_are_redacted() {
        // Assemble the synthetic fixture at compile time.
        let text = concat!(
            "key:\n-----BEGIN OPENSSH ",
            "PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----\ncard 4111 1111 1111 1111 or 4111111111111111"
        );
        let redacted = redact_clipboard(text);
        assert!(!redacted.contains("b3BlbnNzaC1rZXktdjEAAAAA"));
        assert!(!redacted.contains("4111111111111111"));
    }

    #[test]
    fn ordinary_text_is_untouched() {
        let text =
            "Meeting notes: ship the activity tab on Friday, see https://example.com/docs/page";
        assert_eq!(redact_clipboard(text), text);
        assert_eq!(
            redact_clipboard("src/components/settings/AppSettings.tsx"),
            "src/components/settings/AppSettings.tsx"
        );
    }

    #[test]
    fn long_clipboards_are_truncated() {
        let text = "word ".repeat(400);
        let redacted = redact_clipboard(&text);
        assert!(redacted.chars().count() <= CLIPBOARD_MAX_CHARS + TRUNCATED.len());
        assert!(redacted.ends_with(TRUNCATED));
    }
}
