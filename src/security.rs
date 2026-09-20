use crate::error::{Error, Result};

/// Maximum provenance label stored with a memory. Provenance is deliberately
/// compact because it is metadata for audit, not prompt context.
pub const MAX_PROVENANCE_BYTES: usize = 512;

/// Detect only high-confidence credential shapes. This intentionally avoids
/// keyword-only heuristics, which would reject legitimate security guidance.
pub fn detected_secret_kind(value: &str) -> Option<&'static str> {
    if value.contains("-----BEGIN PRIVATE KEY-----")
        || value.contains("-----BEGIN RSA PRIVATE KEY-----")
        || value.contains("-----BEGIN OPENSSH PRIVATE KEY-----")
    {
        return Some("private key");
    }

    for token in
        value.split(|c: char| c.is_ascii_whitespace() || matches!(c, '"' | '\'' | ',' | ';'))
    {
        let trimmed = token.trim_matches(|c: char| matches!(c, '(' | ')' | '[' | ']' | '{' | '}'));
        if is_github_token(trimmed) {
            return Some("GitHub token");
        }
        if is_aws_access_key(trimmed) {
            return Some("AWS access key");
        }
        if is_jwt(trimmed) {
            return Some("JWT");
        }
    }

    for line in value.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed
            .strip_prefix("Authorization: Bearer ")
            .or_else(|| trimmed.strip_prefix("authorization: bearer "))
            && credential_value(rest)
        {
            return Some("bearer token");
        }

        let Some((name, raw_value)) = trimmed.split_once(['=', ':']) else {
            continue;
        };
        let normalized_name = name.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        if matches!(
            normalized_name.as_str(),
            "api_key"
                | "apikey"
                | "access_token"
                | "auth_token"
                | "client_secret"
                | "password"
                | "private_key"
        ) && credential_value(raw_value)
        {
            return Some("credential assignment");
        }
    }

    None
}

pub fn reject_secret(value: &str) -> Result<()> {
    if let Some(kind) = detected_secret_kind(value) {
        return Err(Error::Usage(format!(
            "Memory value appears to contain a {kind}; store a redacted reference instead"
        )));
    }
    Ok(())
}

fn credential_value(raw: &str) -> bool {
    let candidate = raw
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '`'));
    candidate.len() >= 16
        && !candidate.contains(char::is_whitespace)
        && !matches!(
            candidate.to_ascii_lowercase().as_str(),
            "redacted" | "<redacted>" | "changeme" | "example" | "placeholder"
        )
}

fn is_github_token(token: &str) -> bool {
    let prefixes = ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"];
    prefixes.iter().any(|prefix| {
        token.strip_prefix(prefix).is_some_and(|tail| {
            tail.len() >= 30 && tail.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
    })
}

fn is_aws_access_key(token: &str) -> bool {
    token.len() == 20
        && token.starts_with("AKIA")
        && token
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

fn is_jwt(token: &str) -> bool {
    let mut parts = token.split('.');
    let Some(header) = parts.next() else {
        return false;
    };
    let Some(payload) = parts.next() else {
        return false;
    };
    let Some(signature) = parts.next() else {
        return false;
    };
    parts.next().is_none()
        && header.starts_with("eyJ")
        && header.len() >= 12
        && payload.len() >= 12
        && signature.len() >= 16
        && [header, payload, signature].into_iter().all(|part| {
            part.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_high_confidence_secrets_without_blocking_guidance() {
        assert_eq!(
            detected_secret_kind("api_key=abcdefghijklmnopqrstuvwxyz123456"),
            Some("credential assignment")
        );
        assert_eq!(
            detected_secret_kind("Authorization: Bearer abcdefghijklmnopqrstuvwxyz"),
            Some("bearer token")
        );
        assert!(detected_secret_kind("Redact API keys and Bearer tokens before logging").is_none());
        assert!(detected_secret_kind("password=<redacted>").is_none());
    }
}
