use std::collections::HashMap;
use std::path::Path;

use skillfile_core::error::SkillfileError;
use skillfile_core::lock::{lock_key, read_lock};
use skillfile_core::models::{Entry, Manifest, SourceFields};
use skillfile_core::parser::{parse_manifest, MANIFEST_NAME};
use skillfile_deploy::paths::{installed_dir_files, installed_path};
use skillfile_sources::strategy::{content_file, is_dir_entry, meta_sha};
use skillfile_sources::sync::vendor_dir_for;

/// Check if an installed file differs from cache (local only, no network).
fn is_modified_local(entry: &Entry, manifest: &Manifest, repo_root: &Path) -> bool {
    if matches!(entry.source, SourceFields::Local { .. }) {
        return false;
    }

    let result: Result<bool, ()> = (|| {
        if is_dir_entry(entry) {
            return Ok(is_dir_modified_local(entry, manifest, repo_root));
        }

        let dest = installed_path(entry, manifest, repo_root).map_err(|_| ())?;
        if !dest.exists() {
            return Ok(false);
        }

        let vdir = vendor_dir_for(entry, repo_root);
        let cf = content_file(entry);
        if cf.is_empty() {
            return Ok(false);
        }
        let cache_file = vdir.join(&cf);
        if !cache_file.exists() {
            return Ok(false);
        }

        let cache_text = std::fs::read_to_string(&cache_file).map_err(|_| ())?;
        let installed_text = std::fs::read_to_string(&dest).map_err(|_| ())?;

        // Phase 5: pin-aware comparison goes here
        Ok(installed_text != cache_text)
    })();

    result.unwrap_or(false)
}

fn is_dir_modified_local(entry: &Entry, manifest: &Manifest, repo_root: &Path) -> bool {
    let result: Result<bool, ()> = (|| {
        let installed = installed_dir_files(entry, manifest, repo_root).map_err(|_| ())?;
        if installed.is_empty() {
            return Ok(false);
        }

        let vdir = vendor_dir_for(entry, repo_root);
        if !vdir.is_dir() {
            return Ok(false);
        }

        for cache_file in walkdir_files(&vdir) {
            if cache_file.file_name().map_or(true, |n| n == ".meta") {
                continue;
            }
            let filename = cache_file
                .strip_prefix(&vdir)
                .map_err(|_| ())?
                .to_string_lossy()
                .to_string();
            let inst_path = match installed.get(&filename) {
                Some(p) if p.exists() => p,
                _ => continue,
            };

            let cache_text = std::fs::read_to_string(&cache_file).map_err(|_| ())?;
            let installed_text = std::fs::read_to_string(inst_path).map_err(|_| ())?;

            // Phase 5: pin-aware comparison goes here
            if installed_text != cache_text {
                return Ok(true);
            }
        }
        Ok(false)
    })();

    result.unwrap_or(false)
}

fn walkdir_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    walkdir_inner(dir, &mut files);
    files.sort();
    files
}

fn walkdir_inner(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walkdir_inner(&path, files);
            } else {
                files.push(path);
            }
        }
    }
}

pub fn cmd_status(repo_root: &Path, check_upstream: bool) -> Result<(), SkillfileError> {
    let manifest_path = repo_root.join(MANIFEST_NAME);
    if !manifest_path.exists() {
        return Err(SkillfileError::Manifest(format!(
            "{MANIFEST_NAME} not found in {}. Create one and run `skillfile init`.",
            repo_root.display()
        )));
    }

    let result = parse_manifest(&manifest_path)?;
    let manifest = result.manifest;
    let locked = read_lock(repo_root)?;
    let mut sha_cache: HashMap<(String, String), String> = HashMap::new();

    let col_w = manifest
        .entries
        .iter()
        .map(|e| e.name.len())
        .max()
        .unwrap_or(10)
        + 2;

    for entry in &manifest.entries {
        let key = lock_key(entry);
        let name = &entry.name;

        if matches!(entry.source, SourceFields::Local { .. }) {
            println!("{name:<col_w$} local");
            continue;
        }

        let locked_info = match locked.get(&key) {
            Some(li) => li,
            None => {
                println!("{name:<col_w$} unlocked");
                continue;
            }
        };

        let sha = &locked_info.sha;
        let vdir = vendor_dir_for(entry, repo_root);
        let meta = meta_sha(&vdir);

        let mut annotations = Vec::new();
        // Phase 5: [pinned] annotation goes here
        if is_modified_local(entry, &manifest, repo_root) {
            annotations.push("[modified]");
        }
        let annotation = if annotations.is_empty() {
            String::new()
        } else {
            format!("  {}", annotations.join("  "))
        };

        let sha_short = &sha[..12.min(sha.len())];

        let status = if meta.as_deref() != Some(sha.as_str()) {
            format!("locked    sha={sha_short}  (missing or stale){annotation}")
        } else if check_upstream && matches!(entry.source, SourceFields::Github { .. }) {
            let cache_key = (entry.owner_repo().to_string(), entry.ref_().to_string());
            let upstream_sha = if let Some(cached) = sha_cache.get(&cache_key) {
                cached.clone()
            } else {
                let agent = ureq::Agent::new_with_defaults();
                let resolved = skillfile_sources::resolver::resolve_github_sha(
                    &agent,
                    entry.owner_repo(),
                    entry.ref_(),
                )?;
                sha_cache.insert(cache_key, resolved.clone());
                resolved
            };
            if upstream_sha == *sha {
                format!("up to date  sha={sha_short}{annotation}")
            } else {
                let upstream_short = &upstream_sha[..12.min(upstream_sha.len())];
                format!("outdated    locked={sha_short}  upstream={upstream_short}{annotation}")
            }
        } else {
            format!("locked    sha={sha_short}{annotation}")
        };

        println!("{name:<col_w$} {status}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_manifest(dir: &Path, content: &str) {
        std::fs::write(dir.join(MANIFEST_NAME), content).unwrap();
    }

    fn write_lock(dir: &Path, data: &serde_json::Value) {
        std::fs::write(
            dir.join("Skillfile.lock"),
            serde_json::to_string_pretty(data).unwrap(),
        )
        .unwrap();
    }

    fn write_meta(dir: &Path, entity_type: &str, name: &str, sha: &str) {
        let vdir = dir
            .join(".skillfile/cache")
            .join(format!("{entity_type}s"))
            .join(name);
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(
            vdir.join(".meta"),
            serde_json::json!({"sha": sha}).to_string(),
        )
        .unwrap();
    }

    fn write_vendor_content(
        dir: &Path,
        entity_type: &str,
        name: &str,
        filename: &str,
        content: &str,
    ) {
        let vdir = dir
            .join(".skillfile/cache")
            .join(format!("{entity_type}s"))
            .join(name);
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join(filename), content).unwrap();
    }

    const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const ORIGINAL: &str = "# Agent\n\nUpstream content.\n";
    const MODIFIED: &str = "# Agent\n\nUpstream content.\n\n## Custom Section\n\nAdded by user.\n";

    #[test]
    fn no_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let result = cmd_status(dir.path(), false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn local_entry_shows_local() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(dir.path(), "local  skill  foo  skills/foo.md\n");
        // Capture output by running — for unit test we just verify no error
        cmd_status(dir.path(), false).unwrap();
    }

    #[test]
    fn github_entry_unlocked() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "github  agent  my-agent  owner/repo  agents/agent.md  main\n",
        );
        cmd_status(dir.path(), false).unwrap();
    }

    #[test]
    fn github_entry_locked_vendor_matches() {
        let dir = tempfile::tempdir().unwrap();
        let sha = "87321636a1c666283d8f17398b45c2644395044b";
        write_manifest(
            dir.path(),
            "github  agent  my-agent  owner/repo  agents/agent.md  main\n",
        );
        write_lock(
            dir.path(),
            &serde_json::json!({"github/agent/my-agent": {"sha": sha, "raw_url": "https://example.com"}}),
        );
        write_meta(dir.path(), "agent", "my-agent", sha);
        cmd_status(dir.path(), false).unwrap();
    }

    #[test]
    fn github_entry_locked_vendor_missing() {
        let dir = tempfile::tempdir().unwrap();
        let sha = "87321636a1c666283d8f17398b45c2644395044b";
        write_manifest(
            dir.path(),
            "github  agent  my-agent  owner/repo  agents/agent.md  main\n",
        );
        write_lock(
            dir.path(),
            &serde_json::json!({"github/agent/my-agent": {"sha": sha, "raw_url": "https://example.com"}}),
        );
        // No .meta written
        cmd_status(dir.path(), false).unwrap();
    }

    #[test]
    fn modified_shows_for_changed_installed_file() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "install  claude-code  local\ngithub  agent  my-agent  owner/repo  agents/agent.md  main\n",
        );
        write_lock(
            dir.path(),
            &serde_json::json!({"github/agent/my-agent": {"sha": SHA, "raw_url": "https://example.com"}}),
        );
        write_meta(dir.path(), "agent", "my-agent", SHA);
        write_vendor_content(dir.path(), "agent", "my-agent", "agent.md", ORIGINAL);
        let installed = dir.path().join(".claude/agents");
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::write(installed.join("my-agent.md"), MODIFIED).unwrap();

        // is_modified_local should return true
        let result = parse_manifest(&dir.path().join(MANIFEST_NAME)).unwrap();
        let manifest = result.manifest;
        let entry = &manifest.entries[0];
        assert!(is_modified_local(entry, &manifest, dir.path()));
    }

    #[test]
    fn modified_not_shown_for_clean_entry() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "install  claude-code  local\ngithub  agent  my-agent  owner/repo  agents/agent.md  main\n",
        );
        write_lock(
            dir.path(),
            &serde_json::json!({"github/agent/my-agent": {"sha": SHA, "raw_url": "https://example.com"}}),
        );
        write_meta(dir.path(), "agent", "my-agent", SHA);
        write_vendor_content(dir.path(), "agent", "my-agent", "agent.md", ORIGINAL);
        let installed = dir.path().join(".claude/agents");
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::write(installed.join("my-agent.md"), ORIGINAL).unwrap();

        let result = parse_manifest(&dir.path().join(MANIFEST_NAME)).unwrap();
        let manifest = result.manifest;
        let entry = &manifest.entries[0];
        assert!(!is_modified_local(entry, &manifest, dir.path()));
    }

    #[test]
    fn modified_not_shown_when_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "install  claude-code  local\ngithub  agent  my-agent  owner/repo  agents/agent.md  main\n",
        );
        write_lock(
            dir.path(),
            &serde_json::json!({"github/agent/my-agent": {"sha": SHA, "raw_url": "https://example.com"}}),
        );
        write_meta(dir.path(), "agent", "my-agent", SHA);
        write_vendor_content(dir.path(), "agent", "my-agent", "agent.md", ORIGINAL);
        // No installed file

        let result = parse_manifest(&dir.path().join(MANIFEST_NAME)).unwrap();
        let manifest = result.manifest;
        let entry = &manifest.entries[0];
        assert!(!is_modified_local(entry, &manifest, dir.path()));
    }

    #[test]
    fn modified_not_shown_without_vendor_cache() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "install  claude-code  local\ngithub  agent  my-agent  owner/repo  agents/agent.md  main\n",
        );
        write_lock(
            dir.path(),
            &serde_json::json!({"github/agent/my-agent": {"sha": SHA, "raw_url": "https://example.com"}}),
        );
        write_meta(dir.path(), "agent", "my-agent", SHA);
        // No vendor cache content file
        let installed = dir.path().join(".claude/agents");
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::write(installed.join("my-agent.md"), MODIFIED).unwrap();

        let result = parse_manifest(&dir.path().join(MANIFEST_NAME)).unwrap();
        let manifest = result.manifest;
        let entry = &manifest.entries[0];
        assert!(!is_modified_local(entry, &manifest, dir.path()));
    }
}
