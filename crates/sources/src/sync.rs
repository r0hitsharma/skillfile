use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use skillfile_core::error::SkillfileError;
use skillfile_core::lock::{lock_key, read_lock, write_lock};
use skillfile_core::models::{Entry, LockEntry, SourceFields};
use skillfile_core::parser::{parse_manifest, MANIFEST_NAME};

use crate::http::{HttpClient, UreqClient};
use crate::resolver::{
    fetch_files_parallel, fetch_github_file, http_get, list_github_dir_recursive,
    resolve_github_sha,
};
use crate::strategy::{content_file, is_dir_entry, meta_sha};

pub const VENDOR_DIR: &str = ".skillfile/cache";

/// Options for sync/install operations.
pub struct SyncContext {
    pub repo_root: PathBuf,
    pub dry_run: bool,
    pub update: bool,
    pub sha_cache: std::collections::HashMap<(String, String), String>,
}

/// Compute the vendor cache directory for an entry.
pub fn vendor_dir_for(entry: &Entry, repo_root: &Path) -> PathBuf {
    repo_root
        .join(VENDOR_DIR)
        .join(format!("{}s", entry.entity_type))
        .join(&entry.name)
}

/// Check if content exists in the vendor directory for an entry.
fn content_exists(entry: &Entry, vdir: &Path) -> bool {
    match &entry.source {
        SourceFields::Github { .. } => {
            if is_dir_entry(entry) {
                vdir.is_dir()
                    && std::fs::read_dir(vdir)
                        .map(|rd| rd.filter_map(|e| e.ok()).any(|e| e.file_name() != ".meta"))
                        .unwrap_or(false)
            } else {
                let cf = content_file(entry);
                !cf.is_empty() && vdir.join(&cf).exists()
            }
        }
        SourceFields::Local { .. } => false,
        SourceFields::Url { .. } => {
            let cf = content_file(entry);
            !cf.is_empty() && vdir.join(&cf).exists()
        }
    }
}

/// Sync a single entry. Returns updated lock map.
pub fn sync_entry(
    client: &dyn HttpClient,
    entry: &Entry,
    ctx: &mut SyncContext,
    locked: &mut BTreeMap<String, LockEntry>,
) -> Result<(), SkillfileError> {
    match &entry.source {
        SourceFields::Local { .. } => {
            let label = format!(
                "  {}/{}/{}",
                entry.source_type(),
                entry.entity_type,
                entry.name
            );
            eprintln!("{label}: local — skipping");
            Ok(())
        }
        SourceFields::Github { .. } => sync_github(client, entry, ctx, locked),
        SourceFields::Url { url } => sync_url(client, entry, url, ctx, locked),
    }
}

fn sync_github(
    client: &dyn HttpClient,
    entry: &Entry,
    ctx: &mut SyncContext,
    locked: &mut BTreeMap<String, LockEntry>,
) -> Result<(), SkillfileError> {
    let label = format!(
        "  {}/{}/{}",
        entry.source_type(),
        entry.entity_type,
        entry.name
    );
    let vdir = vendor_dir_for(entry, &ctx.repo_root);
    let key = lock_key(entry);

    let SourceFields::Github {
        owner_repo,
        path_in_repo,
        ref_,
    } = &entry.source
    else {
        unreachable!()
    };
    let locked_sha = if ctx.update {
        None
    } else {
        locked.get(&key).map(|le| le.sha.clone())
    };
    let meta = meta_sha(&vdir);
    let has_content = content_exists(entry, &vdir);

    // Skip if locked SHA matches meta and content exists
    if let Some(ref ls) = locked_sha {
        if meta.as_deref() == Some(ls.as_str()) && has_content {
            eprintln!("{label}: up to date (sha={})", &ls[..12.min(ls.len())]);
            return Ok(());
        }
    }

    // Resolve SHA
    let sha = if let Some(ref ls) = locked_sha {
        eprint!(
            "{label}: re-fetching (locked sha={}) ...",
            &ls[..12.min(ls.len())]
        );
        ls.clone()
    } else {
        eprint!("{label}: resolving {owner_repo}@{ref_} ...");
        if ctx.dry_run {
            eprintln!(" [dry-run]");
            return Ok(());
        }
        let cache_key = (owner_repo.to_string(), ref_.to_string());
        if let Some(cached) = ctx.sha_cache.get(&cache_key) {
            let sha = cached.clone();
            eprint!(" sha={} (cached)", &sha[..12.min(sha.len())]);
            sha
        } else {
            let sha = resolve_github_sha(client, owner_repo, ref_)?;
            eprint!(" sha={}", &sha[..12.min(sha.len())]);
            ctx.sha_cache.insert(cache_key, sha.clone());
            sha
        }
    };

    if ctx.dry_run {
        eprintln!(" [dry-run]");
        return Ok(());
    }

    // After resolving SHA on --update, skip download if cache is current
    if ctx.update && meta.as_deref() == Some(sha.as_str()) && has_content {
        eprintln!(" up to date");
        let raw_url = locked
            .get(&key)
            .map(|le| le.raw_url.clone())
            .unwrap_or_default();
        locked.insert(key, LockEntry { sha, raw_url });
        return Ok(());
    }

    // Fetch and write
    std::fs::create_dir_all(&vdir)?;

    let raw_url = if is_dir_entry(entry) {
        let dir_entries = list_github_dir_recursive(client, owner_repo, path_in_repo, &sha)?;
        let fetched = fetch_files_parallel(client, &dir_entries)?;
        for (relative_path, content) in &fetched {
            let dest = vdir.join(relative_path);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dest, content.as_bytes())?;
        }
        eprintln!(" -> {}/ ({} files)", vdir.display(), fetched.len());
        format!("https://api.github.com/repos/{owner_repo}/contents/{path_in_repo}?ref={sha}")
    } else {
        let content = fetch_github_file(client, owner_repo, path_in_repo, &sha)?;
        let effective_path = if path_in_repo == "." {
            "SKILL.md"
        } else {
            path_in_repo
        };
        let filename = std::path::Path::new(effective_path)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("content.md");
        let dest = vdir.join(filename);
        std::fs::write(&dest, &content)?;
        eprintln!(" -> {}", dest.display());
        format!("https://raw.githubusercontent.com/{owner_repo}/{sha}/{effective_path}")
    };

    // Write .meta
    let meta_data = serde_json::json!({
        "source_type": "github",
        "owner_repo": owner_repo,
        "path_in_repo": path_in_repo,
        "ref": ref_,
        "sha": &sha,
        "raw_url": &raw_url,
    });
    std::fs::write(
        vdir.join(".meta"),
        serde_json::to_string_pretty(&meta_data).expect("json! values are always serializable")
            + "\n",
    )?;

    locked.insert(key, LockEntry { sha, raw_url });
    Ok(())
}

fn sync_url(
    client: &dyn HttpClient,
    entry: &Entry,
    url: &str,
    ctx: &SyncContext,
    _locked: &mut BTreeMap<String, LockEntry>,
) -> Result<(), SkillfileError> {
    let label = format!(
        "  {}/{}/{}",
        entry.source_type(),
        entry.entity_type,
        entry.name
    );
    let vdir = vendor_dir_for(entry, &ctx.repo_root);

    eprint!("{label}: fetching {url} ...");

    if ctx.dry_run {
        eprintln!(" [dry-run]");
        return Ok(());
    }

    let content = http_get(client, url)?;
    let filename = std::path::Path::new(url)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("content.md");

    std::fs::create_dir_all(&vdir)?;
    std::fs::write(vdir.join(filename), &content)?;

    let meta_data = serde_json::json!({
        "source_type": "url",
        "url": url,
    });
    std::fs::write(
        vdir.join(".meta"),
        serde_json::to_string_pretty(&meta_data).expect("json! values are always serializable")
            + "\n",
    )?;

    eprintln!(" -> {}", vdir.join(filename).display());
    // URL entries don't have SHA-based locking
    Ok(())
}

/// Run the `sync` command.
pub fn cmd_sync(
    repo_root: &Path,
    dry_run: bool,
    entry_filter: Option<&str>,
    update: bool,
) -> Result<(), SkillfileError> {
    let manifest_path = repo_root.join(MANIFEST_NAME);
    if !manifest_path.exists() {
        return Err(SkillfileError::Manifest(format!(
            "{MANIFEST_NAME} not found in {}. Create one and run `skillfile init`.",
            repo_root.display()
        )));
    }

    let result = parse_manifest(&manifest_path)?;
    for w in &result.warnings {
        eprintln!("{w}");
    }

    let mut entries: Vec<&Entry> = result.manifest.entries.iter().collect();

    if let Some(name) = entry_filter {
        entries.retain(|e| e.name == name);
        if entries.is_empty() {
            return Err(SkillfileError::Manifest(format!(
                "no entry named '{name}' in {MANIFEST_NAME}"
            )));
        }
    }

    if entries.is_empty() {
        eprintln!("No entries found in {MANIFEST_NAME}.");
        return Ok(());
    }

    let mode = if dry_run { " [dry-run]" } else { "" };
    let count = entries.len();
    let noun = if count == 1 { "entry" } else { "entries" };
    eprintln!("Syncing {count} {noun}{mode}...");

    let mut locked = read_lock(repo_root)?;
    let client = UreqClient::new();
    let mut ctx = SyncContext {
        repo_root: repo_root.to_path_buf(),
        dry_run,
        update,
        sha_cache: std::collections::HashMap::new(),
    };

    for entry in &entries {
        sync_entry(&client, entry, &mut ctx, &mut locked)?;
    }

    if !dry_run {
        write_lock(repo_root, &locked)?;
        eprintln!("Done.");
    }

    Ok(())
}

/// Fetch the text content of a single-file github entry at a specific SHA.
/// Used by `diff` and `resolve` in conflict mode.
pub fn fetch_file_at_sha(
    client: &dyn HttpClient,
    entry: &Entry,
    sha: &str,
) -> Result<String, SkillfileError> {
    let SourceFields::Github {
        owner_repo,
        path_in_repo,
        ..
    } = &entry.source
    else {
        return Err(SkillfileError::Network(
            "fetch_file_at_sha only supports github entries".into(),
        ));
    };
    let bytes = crate::resolver::fetch_github_file(client, owner_repo, path_in_repo, sha)?;
    crate::resolver::decode_safe(bytes)
        .map_err(|_| SkillfileError::Network(format!("binary file at sha {sha}")))
}

/// Fetch all text files in a directory github entry at a specific SHA.
/// Returns a map of relative_path -> content.
/// Used by `diff` and `resolve` in conflict mode.
pub fn fetch_dir_at_sha(
    client: &dyn HttpClient,
    entry: &Entry,
    sha: &str,
) -> Result<std::collections::HashMap<String, String>, SkillfileError> {
    let SourceFields::Github {
        owner_repo,
        path_in_repo,
        ..
    } = &entry.source
    else {
        return Err(SkillfileError::Network(
            "fetch_dir_at_sha only supports github entries".into(),
        ));
    };
    let dir_entries =
        crate::resolver::list_github_dir_recursive(client, owner_repo, path_in_repo, sha)?;
    let fetched = crate::resolver::fetch_files_parallel(client, &dir_entries)?;
    let mut result = std::collections::HashMap::new();
    for (path, content) in fetched {
        if let crate::resolver::FileContent::Text(text) = content {
            result.insert(path, text);
        }
        // skip binary files silently
    }
    Ok(result)
}
