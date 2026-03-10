use std::path::Path;

use skillfile_core::error::SkillfileError;
use skillfile_core::lock::{lock_key, read_lock};
use skillfile_core::models::Entry;
use skillfile_core::parser::{find_entry_in, parse_manifest, MANIFEST_NAME};
use skillfile_deploy::install::install_entry;
use skillfile_deploy::paths::{installed_dir_files, installed_path};
use skillfile_sources::strategy::{content_file, is_dir_entry};
use skillfile_sources::sync::vendor_dir_for;

use crate::patch::{
    generate_patch, has_dir_patch, has_patch, remove_all_dir_patches, remove_dir_patch,
    remove_patch, walkdir, write_dir_patch, write_patch,
};

fn pin_dir_entry(entry: &Entry, repo_root: &Path, dry_run: bool) -> Result<String, SkillfileError> {
    let vdir = vendor_dir_for(entry, repo_root);
    if !vdir.is_dir() {
        return Err(SkillfileError::Manifest(format!(
            "'{}' is not cached — run `skillfile install` first",
            entry.name
        )));
    }

    let result = parse_manifest(&repo_root.join(MANIFEST_NAME))?;
    let installed = installed_dir_files(entry, &result.manifest, repo_root)?;
    if installed.is_empty() {
        return Err(SkillfileError::Manifest(format!(
            "'{}' is not installed — run `skillfile install` first",
            entry.name
        )));
    }

    let mut pinned: Vec<String> = Vec::new();

    for cache_file in walkdir(&vdir) {
        if cache_file.file_name().is_some_and(|n| n == ".meta") {
            continue;
        }
        let filename = cache_file
            .strip_prefix(&vdir)
            .ok()
            .and_then(|p| p.to_str())
            .unwrap_or("")
            .to_string();
        if filename.is_empty() {
            continue;
        }
        let inst_path = match installed.get(&filename) {
            Some(p) => p,
            None => continue,
        };
        if !inst_path.exists() {
            continue;
        }
        let original_text = std::fs::read_to_string(&cache_file)?;
        let inst_text = std::fs::read_to_string(inst_path)?;
        let patch_text = generate_patch(&original_text, &inst_text, &filename);

        if !patch_text.is_empty() {
            if !dry_run {
                write_dir_patch(entry, &filename, &patch_text, repo_root)?;
            }
            pinned.push(filename);
        } else if !dry_run {
            remove_dir_patch(entry, &filename, repo_root)?;
        }
    }

    let prefix = if dry_run { "Would pin" } else { "Pinned" };
    if pinned.is_empty() {
        Ok(format!(
            "'{}' matches upstream — nothing to pin",
            entry.name
        ))
    } else {
        Ok(format!("{prefix} '{}' ({})", entry.name, pinned.join(", ")))
    }
}

fn pin_entry(entry: &Entry, repo_root: &Path, dry_run: bool) -> Result<String, SkillfileError> {
    if entry.source_type() == "local" {
        return Ok(format!("'{}' is a local entry — skipped", entry.name));
    }

    let locked = read_lock(repo_root)?;
    let key = lock_key(entry);
    if !locked.contains_key(&key) {
        return Err(SkillfileError::Manifest(format!(
            "'{}' is not locked — run `skillfile install` first",
            entry.name
        )));
    }

    if is_dir_entry(entry) {
        return pin_dir_entry(entry, repo_root, dry_run);
    }

    // Single-file entry
    let vdir = vendor_dir_for(entry, repo_root);
    let cf = content_file(entry);
    let cache_file = if cf.is_empty() {
        return Err(SkillfileError::Manifest(format!(
            "'{}' is not cached — run `skillfile install` first",
            entry.name
        )));
    } else {
        vdir.join(&cf)
    };

    if !cache_file.exists() {
        return Err(SkillfileError::Manifest(format!(
            "'{}' is not cached — run `skillfile install` first",
            entry.name
        )));
    }

    let result = parse_manifest(&repo_root.join(MANIFEST_NAME))?;
    let dest = installed_path(entry, &result.manifest, repo_root)?;
    if !dest.exists() {
        return Err(SkillfileError::Manifest(format!(
            "'{}' is not installed — run `skillfile install` first",
            entry.name
        )));
    }

    let label = format!("{}.md", entry.name);
    let cache_text = std::fs::read_to_string(&cache_file)?;
    let dest_text = std::fs::read_to_string(&dest)?;
    let patch_text = generate_patch(&cache_text, &dest_text, &label);

    if patch_text.is_empty() {
        return Ok(format!(
            "'{}' matches upstream — nothing to pin",
            entry.name
        ));
    }

    if !dry_run {
        write_patch(entry, &patch_text, repo_root)?;
    }
    let prefix = if dry_run { "Would pin" } else { "Pinned" };
    Ok(format!("{prefix} '{}'", entry.name))
}

pub fn cmd_pin(name: &str, repo_root: &Path, dry_run: bool) -> Result<(), SkillfileError> {
    let manifest_path = repo_root.join(MANIFEST_NAME);
    let result = parse_manifest(&manifest_path)?;
    let entry = find_entry_in(name, &result.manifest)?;

    let status = pin_entry(entry, repo_root, dry_run)?;

    if status.starts_with("Pinned") {
        println!("{status} — customisations saved to .skillfile/patches/");
    } else if status.starts_with("Would pin") {
        println!("{status} [dry-run]");
    } else {
        println!("{status}");
    }

    Ok(())
}

pub fn cmd_unpin(name: &str, repo_root: &Path) -> Result<(), SkillfileError> {
    let manifest_path = repo_root.join(MANIFEST_NAME);
    let result = parse_manifest(&manifest_path)?;
    let entry = find_entry_in(name, &result.manifest)?;

    let single = has_patch(entry, repo_root);
    let directory = has_dir_patch(entry, repo_root);

    if !single && !directory {
        println!("'{name}' is not pinned");
        return Ok(());
    }

    if single {
        remove_patch(entry, repo_root)?;
    }
    if directory {
        remove_all_dir_patches(entry, repo_root)?;
    }

    // Restore pristine upstream from vendor cache immediately.
    for target in &result.manifest.install_targets {
        install_entry(entry, target, repo_root, None)?;
    }

    println!("Unpinned '{name}' — restored to upstream version");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use skillfile_core::models::{InstallTarget, Scope, SourceFields};

    fn write_manifest(dir: &Path, content: &str) {
        std::fs::write(dir.join(MANIFEST_NAME), content).unwrap();
    }

    fn write_lock(dir: &Path, content: &str) {
        std::fs::write(dir.join("Skillfile.lock"), content).unwrap();
    }

    fn github_entry_skill(name: &str, path_in_repo: &str) -> Entry {
        Entry {
            entity_type: "skill".into(),
            name: name.to_string(),
            source: SourceFields::Github {
                owner_repo: "owner/repo".into(),
                path_in_repo: path_in_repo.to_string(),
                ref_: "main".into(),
            },
        }
    }

    fn make_lock_json(name: &str, entity_type: &str) -> String {
        format!(
            r#"{{
  "github/{entity_type}/{name}": {{
    "sha": "abc123def456",
    "raw_url": "https://raw.githubusercontent.com/owner/repo/abc123/test.md"
  }}
}}"#
        )
    }

    #[test]
    fn pin_no_manifest_errors() {
        let dir = tempfile::tempdir().unwrap();
        let result = cmd_pin("foo", dir.path(), false);
        assert!(result.is_err());
    }

    #[test]
    fn pin_local_entry_skipped() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(dir.path(), "local  skill  skills/foo.md\n");

        let entry = Entry {
            entity_type: "skill".into(),
            name: "foo".into(),
            source: SourceFields::Local {
                path: "skills/foo.md".into(),
            },
        };
        let result = pin_entry(&entry, dir.path(), false).unwrap();
        assert!(result.contains("local entry — skipped"));
    }

    #[test]
    fn pin_not_locked_errors() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(dir.path(), "github  skill  owner/repo  agents/test.md\n");
        write_lock(dir.path(), "{}");
        let entry = github_entry_skill("test", "agents/test.md");
        let result = pin_entry(&entry, dir.path(), false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not locked"));
    }

    #[test]
    fn pin_not_cached_errors() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(dir.path(), "github  skill  owner/repo  skills/test.md\n");
        write_lock(dir.path(), &make_lock_json("test", "skill"));
        let entry = github_entry_skill("test", "skills/test.md");
        let result = pin_entry(&entry, dir.path(), false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not cached"));
    }

    #[test]
    fn pin_not_installed_errors() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "install  claude-code  local\ngithub  skill  owner/repo  skills/test.md\n",
        );
        write_lock(dir.path(), &make_lock_json("test", "skill"));

        // Create cache but not installed
        let vdir = dir.path().join(".skillfile/cache/skills/test");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("test.md"), "upstream content\n").unwrap();

        let entry = github_entry_skill("test", "skills/test.md");
        let result = pin_entry(&entry, dir.path(), false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not installed"));
    }

    #[test]
    fn pin_matches_upstream_nothing_to_pin() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "install  claude-code  local\ngithub  skill  owner/repo  skills/test.md\n",
        );
        write_lock(dir.path(), &make_lock_json("test", "skill"));

        // Create cache
        let vdir = dir.path().join(".skillfile/cache/skills/test");
        std::fs::create_dir_all(&vdir).unwrap();
        let content = "# Test Skill\n\nSome content.\n";
        std::fs::write(vdir.join("test.md"), content).unwrap();

        // Create installed = same as cache
        let installed_dir = dir.path().join(".claude/skills");
        std::fs::create_dir_all(&installed_dir).unwrap();
        std::fs::write(installed_dir.join("test.md"), content).unwrap();

        let entry = github_entry_skill("test", "skills/test.md");
        let result = pin_entry(&entry, dir.path(), false).unwrap();
        assert!(result.contains("matches upstream — nothing to pin"));
    }

    #[test]
    fn pin_captures_edits() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "install  claude-code  local\ngithub  skill  owner/repo  skills/test.md\n",
        );
        write_lock(dir.path(), &make_lock_json("test", "skill"));

        // Create cache
        let vdir = dir.path().join(".skillfile/cache/skills/test");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("test.md"), "# Test\n\nOriginal content.\n").unwrap();

        // Create installed (modified)
        let installed_dir = dir.path().join(".claude/skills");
        std::fs::create_dir_all(&installed_dir).unwrap();
        std::fs::write(
            installed_dir.join("test.md"),
            "# Test\n\nModified content.\n",
        )
        .unwrap();

        let entry = github_entry_skill("test", "skills/test.md");
        let result = pin_entry(&entry, dir.path(), false).unwrap();
        assert!(result.contains("Pinned 'test'"));

        // Check patch was written
        let patch_path = dir.path().join(".skillfile/patches/skills/test.patch");
        assert!(patch_path.exists());
        let patch_text = std::fs::read_to_string(&patch_path).unwrap();
        assert!(patch_text.contains("-Original content."));
        assert!(patch_text.contains("+Modified content."));
    }

    #[test]
    fn pin_dry_run_does_not_write_patch() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "install  claude-code  local\ngithub  skill  owner/repo  skills/test.md\n",
        );
        write_lock(dir.path(), &make_lock_json("test", "skill"));

        let vdir = dir.path().join(".skillfile/cache/skills/test");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("test.md"), "original\n").unwrap();

        let installed_dir = dir.path().join(".claude/skills");
        std::fs::create_dir_all(&installed_dir).unwrap();
        std::fs::write(installed_dir.join("test.md"), "modified\n").unwrap();

        let entry = github_entry_skill("test", "skills/test.md");
        let result = pin_entry(&entry, dir.path(), true).unwrap();
        assert!(result.contains("Would pin 'test'"));

        // No patch written
        let patch_path = dir.path().join(".skillfile/patches/skills/test.patch");
        assert!(!patch_path.exists());
    }

    #[test]
    fn unpin_not_pinned_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(dir.path(), "github  skill  owner/repo  skills/test.md\n");

        // cmd_unpin uses find_entry_in, which needs the manifest parsed
        // Just check has_patch/has_dir_patch return false
        let entry = github_entry_skill("test", "skills/test.md");
        assert!(!has_patch(&entry, dir.path()));
        assert!(!has_dir_patch(&entry, dir.path()));
    }

    #[test]
    fn unpin_removes_patch() {
        let dir = tempfile::tempdir().unwrap();
        let entry = github_entry_skill("test", "skills/test.md");

        // Write a patch
        write_patch(&entry, "some patch content", dir.path()).unwrap();
        assert!(has_patch(&entry, dir.path()));

        remove_patch(&entry, dir.path()).unwrap();
        assert!(!has_patch(&entry, dir.path()));
    }

    #[test]
    fn pin_dir_entry_not_cached_errors() {
        let dir = tempfile::tempdir().unwrap();
        let entry = Entry {
            entity_type: "agent".into(),
            name: "lang-pro".into(),
            source: SourceFields::Github {
                owner_repo: "owner/repo".into(),
                path_in_repo: "agents/lang-pro".into(), // dir entry
                ref_: "main".into(),
            },
        };
        write_manifest(dir.path(), "");
        let result = pin_dir_entry(&entry, dir.path(), false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not cached"));
    }

    #[test]
    fn cmd_pin_entry_not_found_errors() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(dir.path(), "local  skill  skills/foo.md\n");
        let result = cmd_pin("nonexistent", dir.path(), false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("nonexistent"));
    }

    // Helper: ensure target is set up for installed_path to work
    fn _make_install_target() -> InstallTarget {
        InstallTarget {
            adapter: "claude-code".into(),
            scope: Scope::Local,
        }
    }
}
