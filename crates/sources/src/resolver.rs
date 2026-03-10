use std::process::Command;
use std::sync::OnceLock;

use skillfile_core::error::SkillfileError;

use crate::http::HttpClient;

static TOKEN_CACHE: OnceLock<Option<String>> = OnceLock::new();

/// Discover a GitHub token from environment or `gh` CLI. Cached after first call.
#[must_use]
pub fn github_token() -> Option<&'static str> {
    TOKEN_CACHE
        .get_or_init(|| {
            // Check environment variables first
            if let Ok(token) = std::env::var("GITHUB_TOKEN") {
                if !token.is_empty() {
                    return Some(token);
                }
            }
            if let Ok(token) = std::env::var("GH_TOKEN") {
                if !token.is_empty() {
                    return Some(token);
                }
            }
            // Fall back to `gh auth token`
            match Command::new("gh").args(["auth", "token"]).output() {
                Ok(output) if output.status.success() => {
                    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if token.is_empty() {
                        None
                    } else {
                        Some(token)
                    }
                }
                _ => None,
            }
        })
        .as_deref()
}

/// Perform an HTTP GET and return the response body as bytes.
pub fn http_get(client: &dyn HttpClient, url: &str) -> Result<Vec<u8>, SkillfileError> {
    client.get_bytes(url)
}

/// Try to resolve a git ref to a commit SHA. Returns `None` on 4xx.
fn try_resolve_sha(
    client: &dyn HttpClient,
    owner_repo: &str,
    ref_: &str,
) -> Result<Option<String>, SkillfileError> {
    let url = format!("https://api.github.com/repos/{owner_repo}/commits/{ref_}");
    let text = match client.get_json(&url)? {
        Some(t) => t,
        None => return Ok(None),
    };
    let data: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
        SkillfileError::Network(format!(
            "invalid JSON in SHA response for {owner_repo}@{ref_}: {e}"
        ))
    })?;
    Ok(data["sha"].as_str().map(|s| s.to_string()))
}

/// Resolve a branch/tag/SHA ref to a full commit SHA via GitHub API.
///
/// When ref is `main` and the repo uses `master`, falls back automatically.
pub fn resolve_github_sha(
    client: &dyn HttpClient,
    owner_repo: &str,
    ref_: &str,
) -> Result<String, SkillfileError> {
    if let Some(sha) = try_resolve_sha(client, owner_repo, ref_)? {
        return Ok(sha);
    }
    // Fall back: main <-> master
    let fallback = match ref_ {
        "main" => Some("master"),
        "master" => Some("main"),
        _ => None,
    };
    if let Some(fb) = fallback {
        if let Some(sha) = try_resolve_sha(client, owner_repo, fb)? {
            return Ok(sha);
        }
    }
    Err(SkillfileError::Network(format!(
        "could not resolve {owner_repo}@{ref_}"
    )))
}

/// Fetch raw file bytes from `raw.githubusercontent.com`.
pub fn fetch_github_file(
    client: &dyn HttpClient,
    owner_repo: &str,
    path_in_repo: &str,
    sha: &str,
) -> Result<Vec<u8>, SkillfileError> {
    let effective_path = if path_in_repo == "." {
        "SKILL.md"
    } else {
        path_in_repo
    };
    let url = format!("https://raw.githubusercontent.com/{owner_repo}/{sha}/{effective_path}");
    http_get(client, &url)
}

/// A file entry from a GitHub directory listing.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub relative_path: String,
    pub download_url: String,
}

/// List all files under `base_path` using the Git Trees API.
pub fn list_github_dir_recursive(
    client: &dyn HttpClient,
    owner_repo: &str,
    base_path: &str,
    ref_: &str,
) -> Result<Vec<DirEntry>, SkillfileError> {
    let url = format!("https://api.github.com/repos/{owner_repo}/git/trees/{ref_}?recursive=1");
    let text = client.get_json(&url)?.ok_or_else(|| {
        SkillfileError::Network(format!(
            "failed to list directory {owner_repo}/{base_path}: 4xx error"
        ))
    })?;
    let data: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| SkillfileError::Network(format!("invalid tree JSON: {e}")))?;

    let prefix = format!("{}/", base_path.trim_end_matches('/'));
    let empty = Vec::new();
    let tree = data["tree"].as_array().unwrap_or(&empty);

    let entries = tree
        .iter()
        .filter(|item| {
            item["type"].as_str() == Some("blob")
                && item["path"]
                    .as_str()
                    .is_some_and(|p| p.starts_with(&prefix))
        })
        .filter_map(|item| {
            let path = item["path"].as_str()?;
            let relative_path = path.strip_prefix(&prefix)?.to_string();
            let download_url =
                format!("https://raw.githubusercontent.com/{owner_repo}/{ref_}/{path}");
            Some(DirEntry {
                relative_path,
                download_url,
            })
        })
        .collect();

    Ok(entries)
}

/// Attempt to decode bytes as UTF-8 text. Returns `Err(original_bytes)` for binary.
pub fn decode_safe(raw: Vec<u8>) -> Result<String, Vec<u8>> {
    String::from_utf8(raw).map_err(|e| e.into_bytes())
}

/// File content: either decoded text or raw binary bytes.
#[derive(Debug, Clone)]
pub enum FileContent {
    Text(String),
    Binary(Vec<u8>),
}

impl FileContent {
    pub fn from_bytes(raw: Vec<u8>) -> Self {
        match decode_safe(raw) {
            Ok(text) => FileContent::Text(text),
            Err(bytes) => FileContent::Binary(bytes),
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        match self {
            FileContent::Text(s) => s.as_bytes(),
            FileContent::Binary(b) => b,
        }
    }
}

/// Fetch multiple files in parallel using threads.
pub fn fetch_files_parallel(
    client: &dyn HttpClient,
    files: &[DirEntry],
) -> Result<Vec<(String, FileContent)>, SkillfileError> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    if files.len() == 1 {
        let bytes = http_get(client, &files[0].download_url)?;
        return Ok(vec![(
            files[0].relative_path.clone(),
            FileContent::from_bytes(bytes),
        )]);
    }

    // Parallel fetch using scoped threads (client is &dyn HttpClient: Send + Sync)
    let results: Vec<Result<(String, FileContent), SkillfileError>> = std::thread::scope(|s| {
        let handles: Vec<_> = files
            .iter()
            .map(|entry| {
                let url = entry.download_url.clone();
                let rel = entry.relative_path.clone();
                s.spawn(move || {
                    let bytes = http_get(client, &url)?;
                    Ok((rel, FileContent::from_bytes(bytes)))
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("download thread panicked"))
            .collect()
    });

    results.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_safe_utf8() {
        assert_eq!(
            decode_safe(b"hello world".to_vec()),
            Ok("hello world".to_string())
        );
    }

    #[test]
    fn decode_safe_binary() {
        let binary = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00];
        assert!(decode_safe(binary.clone()).is_err());
        assert_eq!(decode_safe(binary.clone()).unwrap_err(), binary);
    }

    #[test]
    fn file_content_from_bytes_text() {
        let fc = FileContent::from_bytes(b"# Hello".to_vec());
        assert!(matches!(fc, FileContent::Text(_)));
        assert_eq!(fc.as_bytes(), b"# Hello");
    }

    #[test]
    fn file_content_from_bytes_binary() {
        let raw = vec![0x89, 0x50, 0x4E, 0x47];
        let fc = FileContent::from_bytes(raw.clone());
        assert!(matches!(fc, FileContent::Binary(_)));
        assert_eq!(fc.as_bytes(), &raw);
    }
}
