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
    use std::collections::HashMap;

    // -----------------------------------------------------------------------
    // MockClient — in-memory HttpClient for unit tests
    // -----------------------------------------------------------------------

    struct MockClient {
        bytes_responses: HashMap<String, Result<Vec<u8>, SkillfileError>>,
        json_responses: HashMap<String, Result<Option<String>, SkillfileError>>,
    }

    impl MockClient {
        fn new() -> Self {
            Self {
                bytes_responses: HashMap::new(),
                json_responses: HashMap::new(),
            }
        }

        fn add_bytes(&mut self, url: &str, data: Vec<u8>) {
            self.bytes_responses.insert(url.to_string(), Ok(data));
        }

        fn add_bytes_err(&mut self, url: &str, msg: &str) {
            self.bytes_responses.insert(
                url.to_string(),
                Err(SkillfileError::Network(msg.to_string())),
            );
        }

        fn add_json(&mut self, url: &str, json: &str) {
            self.json_responses
                .insert(url.to_string(), Ok(Some(json.to_string())));
        }

        fn add_json_none(&mut self, url: &str) {
            self.json_responses.insert(url.to_string(), Ok(None));
        }

        fn add_json_err(&mut self, url: &str, msg: &str) {
            self.json_responses.insert(
                url.to_string(),
                Err(SkillfileError::Network(msg.to_string())),
            );
        }
    }

    impl HttpClient for MockClient {
        fn get_bytes(&self, url: &str) -> Result<Vec<u8>, SkillfileError> {
            match self.bytes_responses.get(url) {
                Some(Ok(data)) => Ok(data.clone()),
                Some(Err(e)) => Err(SkillfileError::Network(e.to_string())),
                None => Err(SkillfileError::Network(format!(
                    "MockClient: no bytes stub for {url}"
                ))),
            }
        }

        fn get_json(&self, url: &str) -> Result<Option<String>, SkillfileError> {
            match self.json_responses.get(url) {
                Some(Ok(v)) => Ok(v.clone()),
                Some(Err(e)) => Err(SkillfileError::Network(e.to_string())),
                None => Err(SkillfileError::Network(format!(
                    "MockClient: no json stub for {url}"
                ))),
            }
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn sha_json(sha: &str) -> String {
        format!(r#"{{"sha": "{sha}"}}"#)
    }

    fn commit_url(owner_repo: &str, ref_: &str) -> String {
        format!("https://api.github.com/repos/{owner_repo}/commits/{ref_}")
    }

    fn tree_url(owner_repo: &str, ref_: &str) -> String {
        format!("https://api.github.com/repos/{owner_repo}/git/trees/{ref_}?recursive=1")
    }

    // -----------------------------------------------------------------------
    // decode_safe (existing coverage retained)
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // http_get — delegates to client.get_bytes
    // -----------------------------------------------------------------------

    #[test]
    fn http_get_returns_bytes_from_client() {
        let mut client = MockClient::new();
        client.add_bytes("https://example.com/file.md", b"content here".to_vec());

        let result = http_get(&client, "https://example.com/file.md").unwrap();
        assert_eq!(result, b"content here");
    }

    #[test]
    fn http_get_propagates_error() {
        let mut client = MockClient::new();
        client.add_bytes_err("https://example.com/missing.md", "HTTP 404");

        let err = http_get(&client, "https://example.com/missing.md").unwrap_err();
        assert!(err.to_string().contains("HTTP 404"));
    }

    // -----------------------------------------------------------------------
    // try_resolve_sha — private function exercised via resolve_github_sha
    // -----------------------------------------------------------------------

    #[test]
    fn try_resolve_sha_extracts_sha_from_json() {
        // try_resolve_sha is private; exercise it through resolve_github_sha.
        let mut client = MockClient::new();
        let url = commit_url("owner/repo", "main");
        client.add_json(&url, &sha_json("deadbeef1234567890abcdef"));

        let sha = resolve_github_sha(&client, "owner/repo", "main").unwrap();
        assert_eq!(sha, "deadbeef1234567890abcdef");
    }

    #[test]
    fn try_resolve_sha_returns_none_on_4xx() {
        // When the primary ref returns None (4xx) AND the fallback also returns None,
        // resolve_github_sha should produce a Network error.
        let mut client = MockClient::new();
        let primary = commit_url("owner/repo", "v99");
        client.add_json_none(&primary);
        // No fallback registered — "v99" has no main/master alias, so fallback is None.

        let err = resolve_github_sha(&client, "owner/repo", "v99").unwrap_err();
        assert!(err.to_string().contains("could not resolve owner/repo@v99"));
    }

    #[test]
    fn try_resolve_sha_propagates_network_error() {
        let mut client = MockClient::new();
        let url = commit_url("owner/repo", "main");
        client.add_json_err(&url, "connection refused");

        let err = resolve_github_sha(&client, "owner/repo", "main").unwrap_err();
        assert!(err.to_string().contains("connection refused"));
    }

    // -----------------------------------------------------------------------
    // resolve_github_sha — fallback logic
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_github_sha_happy_path() {
        let mut client = MockClient::new();
        let url = commit_url("myorg/myrepo", "main");
        client.add_json(&url, &sha_json("aabbccddeeff00112233445566778899aabbccdd"));

        let sha = resolve_github_sha(&client, "myorg/myrepo", "main").unwrap();
        assert_eq!(sha, "aabbccddeeff00112233445566778899aabbccdd");
    }

    #[test]
    fn resolve_github_sha_main_falls_back_to_master() {
        // Primary "main" returns 4xx (None); fallback "master" succeeds.
        let mut client = MockClient::new();
        client.add_json_none(&commit_url("org/repo", "main"));
        client.add_json(
            &commit_url("org/repo", "master"),
            &sha_json("cafebabe000000000000"),
        );

        let sha = resolve_github_sha(&client, "org/repo", "main").unwrap();
        assert_eq!(sha, "cafebabe000000000000");
    }

    #[test]
    fn resolve_github_sha_master_falls_back_to_main() {
        // Primary "master" returns 4xx (None); fallback "main" succeeds.
        let mut client = MockClient::new();
        client.add_json_none(&commit_url("org/repo", "master"));
        client.add_json(
            &commit_url("org/repo", "main"),
            &sha_json("1234abcd5678ef90"),
        );

        let sha = resolve_github_sha(&client, "org/repo", "master").unwrap();
        assert_eq!(sha, "1234abcd5678ef90");
    }

    #[test]
    fn resolve_github_sha_fails_when_both_branches_absent() {
        // Both "main" and "master" return 4xx — should error.
        let mut client = MockClient::new();
        client.add_json_none(&commit_url("org/repo", "main"));
        client.add_json_none(&commit_url("org/repo", "master"));

        let err = resolve_github_sha(&client, "org/repo", "main").unwrap_err();
        assert!(err.to_string().contains("could not resolve org/repo@main"));
    }

    #[test]
    fn resolve_github_sha_non_main_ref_no_fallback() {
        // A ref like "v1.2.3" that isn't "main"/"master" has no fallback.
        let mut client = MockClient::new();
        client.add_json_none(&commit_url("org/repo", "v1.2.3"));

        let err = resolve_github_sha(&client, "org/repo", "v1.2.3").unwrap_err();
        assert!(err
            .to_string()
            .contains("could not resolve org/repo@v1.2.3"));
    }

    #[test]
    fn resolve_github_sha_invalid_json_returns_error() {
        // JSON that parses successfully but has no "sha" field returns None from
        // try_resolve_sha, propagating through to the "could not resolve" error.
        let mut client = MockClient::new();
        client.add_json(
            &commit_url("org/repo", "main"),
            r#"{"message": "Not Found"}"#,
        );
        // No "sha" key in JSON → try_resolve_sha returns Ok(None).
        client.add_json_none(&commit_url("org/repo", "master"));

        let err = resolve_github_sha(&client, "org/repo", "main").unwrap_err();
        assert!(err.to_string().contains("could not resolve org/repo@main"));
    }

    #[test]
    fn resolve_github_sha_malformed_json_returns_error() {
        let mut client = MockClient::new();
        client.add_json(&commit_url("org/repo", "main"), "not json {{{{");

        let err = resolve_github_sha(&client, "org/repo", "main").unwrap_err();
        assert!(err.to_string().contains("invalid JSON in SHA response"));
    }

    // -----------------------------------------------------------------------
    // fetch_github_file
    // -----------------------------------------------------------------------

    #[test]
    fn fetch_github_file_basic() {
        let sha = "abc123";
        let url = format!("https://raw.githubusercontent.com/owner/repo/{sha}/skills/git.md");
        let mut client = MockClient::new();
        client.add_bytes(&url, b"# Git skill".to_vec());

        let result = fetch_github_file(&client, "owner/repo", "skills/git.md", sha).unwrap();
        assert_eq!(result, b"# Git skill");
    }

    #[test]
    fn fetch_github_file_dot_path_becomes_skill_md() {
        // path_in_repo == "." must be rewritten to "SKILL.md".
        let sha = "def456";
        let url = format!("https://raw.githubusercontent.com/org/repo/{sha}/SKILL.md");
        let mut client = MockClient::new();
        client.add_bytes(&url, b"# Root skill".to_vec());

        let result = fetch_github_file(&client, "org/repo", ".", sha).unwrap();
        assert_eq!(result, b"# Root skill");
    }

    #[test]
    fn fetch_github_file_propagates_error() {
        let sha = "fff000";
        let url = format!("https://raw.githubusercontent.com/org/repo/{sha}/missing.md");
        let mut client = MockClient::new();
        client.add_bytes_err(&url, "HTTP 404: not found");

        let err = fetch_github_file(&client, "org/repo", "missing.md", sha).unwrap_err();
        assert!(err.to_string().contains("HTTP 404: not found"));
    }

    // -----------------------------------------------------------------------
    // list_github_dir_recursive
    // -----------------------------------------------------------------------

    fn tree_json(entries: &[(&str, &str)]) -> String {
        let items: Vec<String> = entries
            .iter()
            .map(|(path, kind)| format!(r#"{{"path": "{path}", "type": "{kind}"}}"#))
            .collect();
        format!(r#"{{"tree": [{}]}}"#, items.join(", "))
    }

    #[test]
    fn list_github_dir_recursive_returns_blobs_under_prefix() {
        let owner_repo = "org/repo";
        let ref_ = "main";
        let url = tree_url(owner_repo, ref_);

        let json = tree_json(&[
            ("agents/dir/file1.md", "blob"),
            ("agents/dir/file2.md", "blob"),
            ("agents/dir/sub", "tree"), // tree entries must be excluded
            ("agents/other/file.md", "blob"), // different prefix — excluded
            ("readme.md", "blob"),      // no prefix at all — excluded
        ]);

        let mut client = MockClient::new();
        client.add_json(&url, &json);

        let entries = list_github_dir_recursive(&client, owner_repo, "agents/dir", ref_).unwrap();

        assert_eq!(entries.len(), 2);

        let relative_paths: Vec<&str> = entries.iter().map(|e| e.relative_path.as_str()).collect();
        assert!(relative_paths.contains(&"file1.md"));
        assert!(relative_paths.contains(&"file2.md"));
    }

    #[test]
    fn list_github_dir_recursive_download_urls_are_correct() {
        let owner_repo = "myorg/myrepo";
        let ref_ = "abc123sha";
        let url = tree_url(owner_repo, ref_);

        let json = tree_json(&[("skills/python/SKILL.md", "blob")]);

        let mut client = MockClient::new();
        client.add_json(&url, &json);

        let entries =
            list_github_dir_recursive(&client, owner_repo, "skills/python", ref_).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].relative_path, "SKILL.md");
        assert_eq!(
            entries[0].download_url,
            format!("https://raw.githubusercontent.com/{owner_repo}/{ref_}/skills/python/SKILL.md")
        );
    }

    #[test]
    fn list_github_dir_recursive_filters_out_tree_nodes() {
        let owner_repo = "org/repo";
        let ref_ = "main";
        let url = tree_url(owner_repo, ref_);

        let json = tree_json(&[
            ("agents/data-analyst", "tree"),
            ("agents/data-analyst/agent.md", "blob"),
        ]);

        let mut client = MockClient::new();
        client.add_json(&url, &json);

        let entries =
            list_github_dir_recursive(&client, owner_repo, "agents/data-analyst", ref_).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].relative_path, "agent.md");
    }

    #[test]
    fn list_github_dir_recursive_empty_tree() {
        let owner_repo = "org/repo";
        let ref_ = "main";
        let url = tree_url(owner_repo, ref_);

        let mut client = MockClient::new();
        client.add_json(&url, r#"{"tree": []}"#);

        let entries = list_github_dir_recursive(&client, owner_repo, "agents/dir", ref_).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn list_github_dir_recursive_4xx_returns_error() {
        let owner_repo = "org/repo";
        let ref_ = "main";
        let url = tree_url(owner_repo, ref_);

        let mut client = MockClient::new();
        client.add_json_none(&url);

        let err = list_github_dir_recursive(&client, owner_repo, "agents/dir", ref_).unwrap_err();
        assert!(err.to_string().contains("failed to list directory"));
    }

    #[test]
    fn list_github_dir_recursive_malformed_json_returns_error() {
        let owner_repo = "org/repo";
        let ref_ = "main";
        let url = tree_url(owner_repo, ref_);

        let mut client = MockClient::new();
        client.add_json(&url, "not valid json");

        let err = list_github_dir_recursive(&client, owner_repo, "agents/dir", ref_).unwrap_err();
        assert!(err.to_string().contains("invalid tree JSON"));
    }

    // -----------------------------------------------------------------------
    // fetch_files_parallel
    // -----------------------------------------------------------------------

    fn make_dir_entry(relative_path: &str, download_url: &str) -> DirEntry {
        DirEntry {
            relative_path: relative_path.to_string(),
            download_url: download_url.to_string(),
        }
    }

    #[test]
    fn fetch_files_parallel_empty_list_returns_empty() {
        let client = MockClient::new();
        let result = fetch_files_parallel(&client, &[]).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn fetch_files_parallel_single_file() {
        let url = "https://raw.githubusercontent.com/org/repo/abc/file.md";
        let mut client = MockClient::new();
        client.add_bytes(url, b"# Single file content".to_vec());

        let files = vec![make_dir_entry("file.md", url)];
        let result = fetch_files_parallel(&client, &files).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "file.md");
        assert_eq!(result[0].1.as_bytes(), b"# Single file content");
    }

    #[test]
    fn fetch_files_parallel_multiple_files() {
        let url1 = "https://raw.githubusercontent.com/org/repo/abc/file1.md";
        let url2 = "https://raw.githubusercontent.com/org/repo/abc/file2.md";
        let url3 = "https://raw.githubusercontent.com/org/repo/abc/file3.md";

        let mut client = MockClient::new();
        client.add_bytes(url1, b"content one".to_vec());
        client.add_bytes(url2, b"content two".to_vec());
        client.add_bytes(url3, b"content three".to_vec());

        let files = vec![
            make_dir_entry("file1.md", url1),
            make_dir_entry("file2.md", url2),
            make_dir_entry("file3.md", url3),
        ];
        let mut result = fetch_files_parallel(&client, &files).unwrap();

        // Sort for deterministic assertion (parallel order not guaranteed).
        result.sort_by(|a, b| a.0.cmp(&b.0));

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].0, "file1.md");
        assert_eq!(result[0].1.as_bytes(), b"content one");
        assert_eq!(result[1].0, "file2.md");
        assert_eq!(result[1].1.as_bytes(), b"content two");
        assert_eq!(result[2].0, "file3.md");
        assert_eq!(result[2].1.as_bytes(), b"content three");
    }

    #[test]
    fn fetch_files_parallel_single_file_text_variant() {
        // Verify TextContent classification works end-to-end through fetch_files_parallel.
        let url = "https://raw.githubusercontent.com/org/repo/abc/skill.md";
        let mut client = MockClient::new();
        client.add_bytes(url, b"# My Skill\n\nDoes things.".to_vec());

        let files = vec![make_dir_entry("skill.md", url)];
        let result = fetch_files_parallel(&client, &files).unwrap();

        assert_eq!(result.len(), 1);
        assert!(matches!(result[0].1, FileContent::Text(_)));
    }

    #[test]
    fn fetch_files_parallel_single_file_binary_variant() {
        let url = "https://raw.githubusercontent.com/org/repo/abc/image.png";
        let mut client = MockClient::new();
        // PNG magic bytes — not valid UTF-8
        client.add_bytes(url, vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);

        let files = vec![make_dir_entry("image.png", url)];
        let result = fetch_files_parallel(&client, &files).unwrap();

        assert_eq!(result.len(), 1);
        assert!(matches!(result[0].1, FileContent::Binary(_)));
    }

    #[test]
    fn fetch_files_parallel_error_propagates_for_single_file() {
        let url = "https://raw.githubusercontent.com/org/repo/abc/missing.md";
        let mut client = MockClient::new();
        client.add_bytes_err(url, "HTTP 404: missing.md not found");

        let files = vec![make_dir_entry("missing.md", url)];
        let err = fetch_files_parallel(&client, &files).unwrap_err();
        assert!(err.to_string().contains("HTTP 404"));
    }

    #[test]
    fn fetch_files_parallel_error_propagates_for_multiple_files() {
        let url1 = "https://raw.githubusercontent.com/org/repo/abc/ok.md";
        let url2 = "https://raw.githubusercontent.com/org/repo/abc/bad.md";
        let url3 = "https://raw.githubusercontent.com/org/repo/abc/also_ok.md";

        let mut client = MockClient::new();
        client.add_bytes(url1, b"ok".to_vec());
        client.add_bytes_err(url2, "HTTP 500: server error");
        client.add_bytes(url3, b"also ok".to_vec());

        let files = vec![
            make_dir_entry("ok.md", url1),
            make_dir_entry("bad.md", url2),
            make_dir_entry("also_ok.md", url3),
        ];
        let err = fetch_files_parallel(&client, &files).unwrap_err();
        assert!(err.to_string().contains("HTTP 500"));
    }
}
