use crate::{AgentContextBlock, AgentContextPreview};

const REDACTED: &str = "[REDACTED]";
const MARKERS: &[&str] = &[
    "token=",
    "password=",
    "secret=",
    "api_key=",
    "authorization=",
    "bearer ",
    "accountkey=",
    "aws_secret_access_key=",
    "aws_session_token=",
    "azure_client_secret=",
    "google_api_key=",
];
const PRIVATE_KEY_KINDS: &[&str] = &[
    "OPENSSH PRIVATE KEY",
    "RSA PRIVATE KEY",
    "EC PRIVATE KEY",
    "DSA PRIVATE KEY",
    "PRIVATE KEY",
];
const SSH_PUBLIC_KEY_PREFIXES: &[&str] = &[
    "ssh-ed25519 ",
    "ssh-rsa ",
    "ecdsa-sha2-nistp256 ",
    "ecdsa-sha2-nistp384 ",
    "ecdsa-sha2-nistp521 ",
];

pub fn redact_secret_text(text: &str) -> String {
    let mut redacted = text.to_string();
    redacted = redact_private_key_blocks(&redacted);
    redacted = redact_ssh_public_key_lines(&redacted);
    for marker in MARKERS {
        redacted = redact_after_marker_case_insensitive(&redacted, marker);
    }
    redact_sensitive_tokens(&redacted)
}

pub fn redact_agent_context_preview(preview: &AgentContextPreview) -> AgentContextPreview {
    AgentContextPreview {
        current_block: preview
            .current_block
            .as_ref()
            .map(redact_agent_context_block),
        selection: preview.selection.as_deref().map(redact_secret_text),
        cwd: preview.cwd.clone(),
        explicit_files: preview.explicit_files.clone(),
    }
}

fn redact_agent_context_block(block: &AgentContextBlock) -> AgentContextBlock {
    AgentContextBlock {
        command: block.command.as_deref().map(redact_secret_text),
        output: redact_secret_text(&block.output),
        exit_code: block.exit_code,
    }
}

fn redact_private_key_blocks(text: &str) -> String {
    let mut redacted = text.to_string();
    for kind in PRIVATE_KEY_KINDS {
        let begin = format!("-----BEGIN {kind}-----");
        let end = format!("-----END {kind}-----");
        redacted = redact_between_markers(&redacted, &begin, &end, "[REDACTED PRIVATE KEY]");
    }
    redacted
}

fn redact_between_markers(text: &str, begin: &str, end: &str, replacement: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;

    while let Some(found_begin) = text[cursor..].find(begin) {
        let start = cursor + found_begin;
        result.push_str(&text[cursor..start]);
        let search_from = start + begin.len();
        if let Some(found_end) = text[search_from..].find(end) {
            let end_index = search_from + found_end + end.len();
            result.push_str(replacement);
            cursor = end_index;
        } else {
            result.push_str(&text[start..]);
            return result;
        }
    }

    result.push_str(&text[cursor..]);
    result
}

fn redact_ssh_public_key_lines(text: &str) -> String {
    let mut result = String::with_capacity(text.len());

    for chunk in text.split_inclusive('\n') {
        let line = chunk.strip_suffix('\n').unwrap_or(chunk);
        let newline = if chunk.ends_with('\n') { "\n" } else { "" };
        let trimmed = line.trim_start();
        if let Some(prefix) = SSH_PUBLIC_KEY_PREFIXES
            .iter()
            .find(|prefix| trimmed.starts_with(**prefix))
        {
            result.push_str(prefix.trim_end());
            result.push(' ');
            result.push_str(REDACTED);
            result.push_str(newline);
        } else {
            result.push_str(line);
            result.push_str(newline);
        }
    }

    result
}

fn redact_after_marker_case_insensitive(text: &str, marker: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let marker = marker.to_ascii_lowercase();
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;

    while let Some(found) = lower[cursor..].find(&marker) {
        let start = cursor + found;
        let value_start = start + marker.len();
        result.push_str(&text[cursor..value_start]);
        let value_end = find_secret_value_end(text, value_start);
        result.push_str(REDACTED);
        cursor = value_end;
    }

    result.push_str(&text[cursor..]);
    result
}

fn find_secret_value_end(text: &str, value_start: usize) -> usize {
    let lower = text[value_start..].to_ascii_lowercase();
    let secret_start = if lower.starts_with("bearer ") {
        value_start + "bearer ".len()
    } else {
        value_start
    };
    text[secret_start..]
        .find(is_secret_delimiter)
        .map(|offset| secret_start + offset)
        .unwrap_or(text.len())
}

fn redact_sensitive_tokens(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut token_start = None;

    for (index, ch) in text.char_indices() {
        if is_token_char(ch) {
            token_start.get_or_insert(index);
            continue;
        }

        if let Some(start) = token_start.take() {
            push_redacted_token(&mut result, &text[start..index]);
        }
        result.push(ch);
    }

    if let Some(start) = token_start {
        push_redacted_token(&mut result, &text[start..]);
    }

    result
}

fn push_redacted_token(result: &mut String, token: &str) {
    let trimmed_len = token.trim_end_matches(['.', ',', ':']).len();
    let (core, suffix) = token.split_at(trimmed_len);
    if is_sensitive_token(core) {
        result.push_str(REDACTED);
        result.push_str(suffix);
    } else {
        result.push_str(token);
    }
}

fn is_sensitive_token(token: &str) -> bool {
    let token = token.trim();
    if token.is_empty() {
        return false;
    }

    token.starts_with("sk-")
        || token.starts_with("ghp_")
        || is_aws_access_key(token)
        || is_google_api_key(token)
        || is_jwt(token)
}

fn is_aws_access_key(token: &str) -> bool {
    let prefix = token.starts_with("AKIA") || token.starts_with("ASIA");
    prefix
        && token.len() == 20
        && token
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
}

fn is_google_api_key(token: &str) -> bool {
    token.starts_with("AIza")
        && token.len() >= 35
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
}

fn is_jwt(token: &str) -> bool {
    let mut parts = token.split('.');
    let Some(first) = parts.next() else {
        return false;
    };
    let Some(second) = parts.next() else {
        return false;
    };
    let Some(third) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }

    first.starts_with("eyJ")
        && [first, second, third]
            .into_iter()
            .all(|part| !part.is_empty() && part.chars().all(is_base64url_char))
}

fn is_base64url_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_')
}

fn is_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | '+')
}

fn is_secret_delimiter(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            ',' | ';' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | '\r' | '\n'
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn redaction_corpus_masks_phase9_secrets() {
        let input = concat!(
            "token=abc123 ",
            "password=hunter2 ",
            "Authorization=Bearer super-secret-token ",
            "gh=ghp_exampletoken1234567890 ",
            "openai=sk-live-secretvalue12345 ",
            "jwt=eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTYifQ.signaturepart ",
            "aws=AKIAIOSFODNN7EXAMPLE ",
            "gcp=AIzaSy012345678901234567890123456789012 ",
            "AccountKey=azure-storage-secret==\n",
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBrW5kW9fNivfK4hY9Dc1Rc1j8G3kQ== user@host\n",
            "-----BEGIN OPENSSH PRIVATE KEY-----\n",
            "super secret private key body\n",
            "-----END OPENSSH PRIVATE KEY-----\n"
        );

        let redacted = redact_secret_text(input);
        for secret in [
            "abc123",
            "hunter2",
            "super-secret-token",
            "ghp_exampletoken1234567890",
            "sk-live-secretvalue12345",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTYifQ.signaturepart",
            "AKIAIOSFODNN7EXAMPLE",
            "AIzaSy012345678901234567890123456789012",
            "azure-storage-secret==",
            "AAAAC3NzaC1lZDI1NTE5AAAAIBrW5kW9fNivfK4hY9Dc1Rc1j8G3kQ==",
            "super secret private key body",
        ] {
            assert!(
                !redacted.contains(secret),
                "redaction leaked {secret:?}: {redacted}"
            );
        }
        assert!(redacted.contains(REDACTED));
        assert!(redacted.contains("[REDACTED PRIVATE KEY]"));
    }

    #[test]
    fn agent_context_preview_redacts_block_and_selection_text() {
        let preview = AgentContextPreview {
            current_block: Some(AgentContextBlock {
                command: Some("echo sk-live-secretvalue12345".to_string()),
                output: "jwt eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTYifQ.signaturepart".to_string(),
                exit_code: Some(0),
            }),
            selection: Some("token=abc123".to_string()),
            cwd: Some(PathBuf::from("/tmp/noctrail")),
            explicit_files: vec![PathBuf::from("/tmp/noctrail/Cargo.toml")],
        };

        let redacted = redact_agent_context_preview(&preview);
        assert_eq!(redacted.cwd, preview.cwd);
        assert_eq!(redacted.explicit_files, preview.explicit_files);
        assert!(
            redacted
                .current_block
                .as_ref()
                .and_then(|block| block.command.as_deref())
                .is_some_and(|command| command.contains(REDACTED))
        );
        assert!(
            redacted
                .current_block
                .as_ref()
                .is_some_and(|block| block.output.contains(REDACTED))
        );
        assert_eq!(redacted.selection.as_deref(), Some("token=[REDACTED]"));
    }
}
