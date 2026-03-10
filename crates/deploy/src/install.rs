use std::collections::HashMap;
use std::path::{Path, PathBuf};

use skillfile_core::conflict::{read_conflict, write_conflict};
use skillfile_core::error::SkillfileError;
use skillfile_core::lock::{lock_key, read_lock};
use skillfile_core::models::{
    short_sha, ConflictState, Entry, InstallOptions, InstallTarget, Manifest,
};
use skillfile_core::parser::{parse_manifest, MANIFEST_NAME};
use skillfile_core::patch::{
    apply_patch_pure, dir_patch_path, generate_patch, has_patch, patches_root, read_patch,
    remove_patch, walkdir, write_dir_patch, write_patch,
};
use skillfile_sources::strategy::{content_file, is_dir_entry};
use skillfile_sources::sync::{cmd_sync, vendor_dir_for};

use crate::adapter::adapters;
use crate::paths::{installed_dir_files, installed_path, source_path};

// ---------------------------------------------------------------------------
// Patch application helpers
// ---------------------------------------------------------------------------

/// Convert a patch application error into PatchConflict for the given entry.
fn to_patch_conflict(err: SkillfileError, entry_name: &str) -> SkillfileError {
    SkillfileError::PatchConflict {
        message: err.to_string(),
        entry_name: entry_name.to_string(),
    }
}

/// Apply stored patch (if any) to a single installed file, then rebase the patch
/// against the new cache content so status comparisons remain correct.
fn apply_single_file_patch(
    entry: &Entry,
    dest: &Path,
    source: &Path,
    repo_root: &Path,
) -> Result<(), SkillfileError> {
    if !has_patch(entry, repo_root) {
        return Ok(());
    }
    let patch_text = read_patch(entry, repo_root)?;
    let original = std::fs::read_to_string(dest)?;
    let patched =
        apply_patch_pure(&original, &patch_text).map_err(|e| to_patch_conflict(e, &entry.name))?;
    std::fs::write(dest, &patched)?;

    // Rebase: regenerate patch against new cache so `diff` shows accurate deltas.
    let cache_text = std::fs::read_to_string(source)?;
    let new_patch = generate_patch(&cache_text, &patched, &format!("{}.md", entry.name));
    if !new_patch.is_empty() {
        write_patch(entry, &new_patch, repo_root)?;
    } else {
        remove_patch(entry, repo_root)?;
    }
    Ok(())
}

/// Apply per-file patches to all installed files of a directory entry.
/// Rebases each patch against the new cache content after applying.
fn apply_dir_patches(
    entry: &Entry,
    installed_files: &HashMap<String, PathBuf>,
    source_dir: &Path,
    repo_root: &Path,
) -> Result<(), SkillfileError> {
    let patches_dir = patches_root(repo_root)
        .join(entry.entity_type.dir_name())
        .join(&entry.name);
    if !patches_dir.is_dir() {
        return Ok(());
    }

    let patch_files: Vec<PathBuf> = walkdir(&patches_dir)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "patch"))
        .collect();

    for patch_file in patch_files {
        let rel = match patch_file
            .strip_prefix(&patches_dir)
            .ok()
            .and_then(|p| p.to_str())
            .and_then(|s| s.strip_suffix(".patch"))
        {
            Some(s) => s.to_string(),
            None => continue,
        };

        let target = match installed_files.get(&rel) {
            Some(p) if p.exists() => p,
            _ => continue,
        };

        let patch_text = std::fs::read_to_string(&patch_file)?;
        let original = std::fs::read_to_string(target)?;
        let patched = apply_patch_pure(&original, &patch_text)
            .map_err(|e| to_patch_conflict(e, &entry.name))?;
        std::fs::write(target, &patched)?;

        let cache_file = source_dir.join(&rel);
        if cache_file.exists() {
            let cache_text = std::fs::read_to_string(&cache_file)?;
            let new_patch = generate_patch(&cache_text, &patched, &rel);
            if !new_patch.is_empty() {
                write_dir_patch(entry, &rel, &new_patch, repo_root)?;
            } else {
                std::fs::remove_file(&patch_file)?;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Auto-pin helpers (used by install --update)
// ---------------------------------------------------------------------------

/// Compare installed vs cache; write patch if they differ. Silent on missing prerequisites.
fn auto_pin_entry(entry: &Entry, manifest: &Manifest, repo_root: &Path) {
    if entry.source_type() == "local" {
        return;
    }

    let locked = match read_lock(repo_root) {
        Ok(l) => l,
        Err(_) => return,
    };
    let key = lock_key(entry);
    if !locked.contains_key(&key) {
        return;
    }

    let vdir = vendor_dir_for(entry, repo_root);

    if is_dir_entry(entry) {
        auto_pin_dir_entry(entry, manifest, repo_root, &vdir);
        return;
    }

    let cf = content_file(entry);
    if cf.is_empty() {
        return;
    }
    let cache_file = vdir.join(&cf);
    if !cache_file.exists() {
        return;
    }

    let dest = match installed_path(entry, manifest, repo_root) {
        Ok(p) => p,
        Err(_) => return,
    };
    if !dest.exists() {
        return;
    }

    let cache_text = match std::fs::read_to_string(&cache_file) {
        Ok(s) => s,
        Err(_) => return,
    };
    let installed_text = match std::fs::read_to_string(&dest) {
        Ok(s) => s,
        Err(_) => return,
    };

    // If already pinned, check if stored patch still describes the installed content exactly.
    if has_patch(entry, repo_root) {
        if let Ok(pt) = read_patch(entry, repo_root) {
            match apply_patch_pure(&cache_text, &pt) {
                Ok(expected) if installed_text == expected => return, // no new edits
                Ok(_) => {} // installed has additional edits — fall through to re-pin
                Err(_) => return, // cache inconsistent with stored patch — preserve
            }
        }
    }

    let patch_text = generate_patch(&cache_text, &installed_text, &format!("{}.md", entry.name));
    if !patch_text.is_empty() && write_patch(entry, &patch_text, repo_root).is_ok() {
        eprintln!(
            "  {}: local changes auto-saved to .skillfile/patches/",
            entry.name
        );
    }
}

/// Auto-pin each modified file in a directory entry's installed copy.
fn auto_pin_dir_entry(entry: &Entry, manifest: &Manifest, repo_root: &Path, vdir: &Path) {
    if !vdir.is_dir() {
        return;
    }

    let installed = match installed_dir_files(entry, manifest, repo_root) {
        Ok(m) => m,
        Err(_) => return,
    };
    if installed.is_empty() {
        return;
    }

    let mut pinned: Vec<String> = Vec::new();
    for cache_file in walkdir(vdir) {
        if cache_file.file_name().is_some_and(|n| n == ".meta") {
            continue;
        }
        let filename = match cache_file.strip_prefix(vdir).ok().and_then(|p| p.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let inst_path = match installed.get(&filename) {
            Some(p) if p.exists() => p,
            _ => continue,
        };

        let cache_text = match std::fs::read_to_string(&cache_file) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let installed_text = match std::fs::read_to_string(inst_path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        // Check if stored dir patch still matches
        let p = dir_patch_path(entry, &filename, repo_root);
        if p.exists() {
            if let Ok(pt) = std::fs::read_to_string(&p) {
                match apply_patch_pure(&cache_text, &pt) {
                    Ok(expected) if installed_text == expected => continue, // no new edits
                    Ok(_) => {}         // fall through to re-pin
                    Err(_) => continue, // cache inconsistent — preserve
                }
            }
        }

        let patch_text = generate_patch(&cache_text, &installed_text, &filename);
        if !patch_text.is_empty()
            && write_dir_patch(entry, &filename, &patch_text, repo_root).is_ok()
        {
            pinned.push(filename);
        }
    }

    if !pinned.is_empty() {
        eprintln!(
            "  {}: local changes auto-saved to .skillfile/patches/ ({})",
            entry.name,
            pinned.join(", ")
        );
    }
}

// ---------------------------------------------------------------------------
// Core install entry point
// ---------------------------------------------------------------------------

/// Deploy one entry to its installed path via the platform adapter.
///
/// The adapter owns all platform-specific logic (target dirs, flat vs. nested).
/// This function handles cross-cutting concerns: source resolution,
/// missing-source warnings, and patch application.
///
/// Returns `Err(PatchConflict)` if a stored patch fails to apply cleanly.
pub fn install_entry(
    entry: &Entry,
    target: &InstallTarget,
    repo_root: &Path,
    opts: Option<&InstallOptions>,
) -> Result<(), SkillfileError> {
    let default_opts = InstallOptions::default();
    let opts = opts.unwrap_or(&default_opts);

    let all_adapters = adapters();
    let adapter = match all_adapters.get(&target.adapter) {
        Some(a) => a,
        None => return Ok(()),
    };

    if !adapter.supports(entry.entity_type.as_str()) {
        return Ok(());
    }

    let source = match source_path(entry, repo_root) {
        Some(p) if p.exists() => p,
        _ => {
            eprintln!("  warning: source missing for {}, skipping", entry.name);
            return Ok(());
        }
    };

    let is_dir = is_dir_entry(entry);
    let installed = adapter.deploy_entry(entry, &source, target.scope, repo_root, opts);

    if !installed.is_empty() && !opts.dry_run {
        if is_dir {
            apply_dir_patches(entry, &installed, &source, repo_root)?;
        } else {
            let key = format!("{}.md", entry.name);
            if let Some(dest) = installed.get(&key) {
                apply_single_file_patch(entry, dest, &source, repo_root)?;
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Precondition check
// ---------------------------------------------------------------------------

fn check_preconditions(manifest: &Manifest, repo_root: &Path) -> Result<(), SkillfileError> {
    if manifest.install_targets.is_empty() {
        return Err(SkillfileError::Manifest(
            "No install targets configured. Run `skillfile init` first.".into(),
        ));
    }

    if let Some(conflict) = read_conflict(repo_root)? {
        return Err(SkillfileError::Install(format!(
            "pending conflict for '{}' — \
             run `skillfile diff {}` to review, \
             or `skillfile resolve {}` to merge",
            conflict.entry, conflict.entry, conflict.entry
        )));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Deploy all entries, handling patch conflicts
// ---------------------------------------------------------------------------

fn deploy_all(
    manifest: &Manifest,
    repo_root: &Path,
    opts: &InstallOptions,
    locked: &std::collections::BTreeMap<String, skillfile_core::models::LockEntry>,
    old_locked: &std::collections::BTreeMap<String, skillfile_core::models::LockEntry>,
) -> Result<(), SkillfileError> {
    let mode = if opts.dry_run { " [dry-run]" } else { "" };
    let all_adapters = adapters();

    for target in &manifest.install_targets {
        if !all_adapters.contains(&target.adapter) {
            eprintln!("warning: unknown platform '{}', skipping", target.adapter);
            continue;
        }
        eprintln!(
            "Installing for {} ({}){mode}...",
            target.adapter, target.scope
        );
        for entry in &manifest.entries {
            match install_entry(entry, target, repo_root, Some(opts)) {
                Ok(()) => {}
                Err(SkillfileError::PatchConflict { entry_name, .. }) => {
                    let key = lock_key(entry);
                    let old_sha = old_locked
                        .get(&key)
                        .map(|l| l.sha.clone())
                        .unwrap_or_default();
                    let new_sha = locked
                        .get(&key)
                        .map(|l| l.sha.clone())
                        .unwrap_or_else(|| old_sha.clone());

                    write_conflict(
                        repo_root,
                        &ConflictState {
                            entry: entry_name.clone(),
                            entity_type: entry.entity_type.to_string(),
                            old_sha: old_sha.clone(),
                            new_sha: new_sha.clone(),
                        },
                    )?;

                    let sha_info =
                        if !old_sha.is_empty() && !new_sha.is_empty() && old_sha != new_sha {
                            format!(
                                "\n  upstream: {} \u{2192} {}",
                                short_sha(&old_sha),
                                short_sha(&new_sha)
                            )
                        } else {
                            String::new()
                        };

                    return Err(SkillfileError::Install(format!(
                        "upstream changes to '{entry_name}' conflict with your customisations.{sha_info}\n\
                         Your pinned edits could not be applied to the new upstream version.\n\
                         Run `skillfile diff {entry_name}` to review what changed upstream.\n\
                         Run `skillfile resolve {entry_name}` when ready to merge.\n\
                         Run `skillfile resolve --abort` to discard the conflict and keep the old version."
                    )));
                }
                Err(e) => return Err(e),
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// cmd_install
// ---------------------------------------------------------------------------

pub fn cmd_install(repo_root: &Path, dry_run: bool, update: bool) -> Result<(), SkillfileError> {
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
    let manifest = result.manifest;

    check_preconditions(&manifest, repo_root)?;

    // Read old locked state before sync (used for SHA context in conflict messages).
    let old_locked = read_lock(repo_root).unwrap_or_default();

    // Auto-pin local edits before re-fetching upstream (--update only).
    if update && !dry_run {
        for entry in &manifest.entries {
            auto_pin_entry(entry, &manifest, repo_root);
        }
    }

    // Fetch any missing or stale entries.
    cmd_sync(repo_root, dry_run, None, update)?;

    // Read new locked state (written by sync).
    let locked = read_lock(repo_root).unwrap_or_default();

    // Deploy to all configured platform targets.
    let opts = InstallOptions {
        dry_run,
        overwrite: update,
    };
    deploy_all(&manifest, repo_root, &opts, &locked, &old_locked)?;

    if !dry_run {
        eprintln!("Done.");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use skillfile_core::models::{EntityType, Entry, InstallTarget, Scope, SourceFields};

    fn make_agent_entry(name: &str) -> Entry {
        Entry {
            entity_type: EntityType::Agent,
            name: name.into(),
            source: SourceFields::Github {
                owner_repo: "owner/repo".into(),
                path_in_repo: "agents/agent.md".into(),
                ref_: "main".into(),
            },
        }
    }

    fn make_local_entry(name: &str, path: &str) -> Entry {
        Entry {
            entity_type: EntityType::Skill,
            name: name.into(),
            source: SourceFields::Local { path: path.into() },
        }
    }

    fn make_target(adapter: &str, scope: Scope) -> InstallTarget {
        InstallTarget {
            adapter: adapter.into(),
            scope,
        }
    }

    // -- install_entry: local source --

    #[test]
    fn install_local_entry_copy() {
        let dir = tempfile::tempdir().unwrap();
        let source_file = dir.path().join("skills/my-skill.md");
        std::fs::create_dir_all(source_file.parent().unwrap()).unwrap();
        std::fs::write(&source_file, "# My Skill").unwrap();

        let entry = make_local_entry("my-skill", "skills/my-skill.md");
        let target = make_target("claude-code", Scope::Local);
        install_entry(&entry, &target, dir.path(), None).unwrap();

        let dest = dir.path().join(".claude/skills/my-skill.md");
        assert!(dest.exists());
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "# My Skill");
    }

    #[test]
    fn install_entry_dry_run_no_write() {
        let dir = tempfile::tempdir().unwrap();
        let source_file = dir.path().join("skills/my-skill.md");
        std::fs::create_dir_all(source_file.parent().unwrap()).unwrap();
        std::fs::write(&source_file, "# My Skill").unwrap();

        let entry = make_local_entry("my-skill", "skills/my-skill.md");
        let target = make_target("claude-code", Scope::Local);
        let opts = InstallOptions {
            dry_run: true,
            ..Default::default()
        };
        install_entry(&entry, &target, dir.path(), Some(&opts)).unwrap();

        let dest = dir.path().join(".claude/skills/my-skill.md");
        assert!(!dest.exists());
    }

    #[test]
    fn install_entry_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let source_file = dir.path().join("skills/my-skill.md");
        std::fs::create_dir_all(source_file.parent().unwrap()).unwrap();
        std::fs::write(&source_file, "# New content").unwrap();

        let dest_dir = dir.path().join(".claude/skills");
        std::fs::create_dir_all(&dest_dir).unwrap();
        let dest = dest_dir.join("my-skill.md");
        std::fs::write(&dest, "# Old content").unwrap();

        let entry = make_local_entry("my-skill", "skills/my-skill.md");
        let target = make_target("claude-code", Scope::Local);
        install_entry(&entry, &target, dir.path(), None).unwrap();

        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "# New content");
    }

    // -- install_entry: github (vendored) source --

    #[test]
    fn install_github_entry_copy() {
        let dir = tempfile::tempdir().unwrap();
        let vdir = dir.path().join(".skillfile/cache/agents/my-agent");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("agent.md"), "# Agent").unwrap();

        let entry = make_agent_entry("my-agent");
        let target = make_target("claude-code", Scope::Local);
        install_entry(&entry, &target, dir.path(), None).unwrap();

        let dest = dir.path().join(".claude/agents/my-agent.md");
        assert!(dest.exists());
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "# Agent");
    }

    #[test]
    fn install_github_dir_entry_copy() {
        let dir = tempfile::tempdir().unwrap();
        let vdir = dir.path().join(".skillfile/cache/skills/python-pro");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("SKILL.md"), "# Python Pro").unwrap();
        std::fs::write(vdir.join("examples.md"), "# Examples").unwrap();

        let entry = Entry {
            entity_type: EntityType::Skill,
            name: "python-pro".into(),
            source: SourceFields::Github {
                owner_repo: "owner/repo".into(),
                path_in_repo: "skills/python-pro".into(),
                ref_: "main".into(),
            },
        };
        let target = make_target("claude-code", Scope::Local);
        install_entry(&entry, &target, dir.path(), None).unwrap();

        let dest = dir.path().join(".claude/skills/python-pro");
        assert!(dest.is_dir());
        assert_eq!(
            std::fs::read_to_string(dest.join("SKILL.md")).unwrap(),
            "# Python Pro"
        );
    }

    #[test]
    fn install_agent_dir_entry_explodes_to_individual_files() {
        let dir = tempfile::tempdir().unwrap();
        let vdir = dir.path().join(".skillfile/cache/agents/core-dev");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("backend-developer.md"), "# Backend").unwrap();
        std::fs::write(vdir.join("frontend-developer.md"), "# Frontend").unwrap();
        std::fs::write(vdir.join(".meta"), "{}").unwrap();

        let entry = Entry {
            entity_type: EntityType::Agent,
            name: "core-dev".into(),
            source: SourceFields::Github {
                owner_repo: "owner/repo".into(),
                path_in_repo: "categories/core-dev".into(),
                ref_: "main".into(),
            },
        };
        let target = make_target("claude-code", Scope::Local);
        install_entry(&entry, &target, dir.path(), None).unwrap();

        let agents_dir = dir.path().join(".claude/agents");
        assert_eq!(
            std::fs::read_to_string(agents_dir.join("backend-developer.md")).unwrap(),
            "# Backend"
        );
        assert_eq!(
            std::fs::read_to_string(agents_dir.join("frontend-developer.md")).unwrap(),
            "# Frontend"
        );
        // No "core-dev" directory should exist — flat mode
        assert!(!agents_dir.join("core-dev").exists());
    }

    #[test]
    fn install_entry_missing_source_warns() {
        let dir = tempfile::tempdir().unwrap();
        let entry = make_agent_entry("my-agent");
        let target = make_target("claude-code", Scope::Local);

        // Should return Ok without error — just a warning
        install_entry(&entry, &target, dir.path(), None).unwrap();
    }

    // -- Patch application during install --

    #[test]
    fn install_applies_existing_patch() {
        let dir = tempfile::tempdir().unwrap();

        // Set up cache
        let vdir = dir.path().join(".skillfile/cache/skills/test");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("test.md"), "# Test\n\nOriginal.\n").unwrap();

        // Write a patch
        let entry = Entry {
            entity_type: EntityType::Skill,
            name: "test".into(),
            source: SourceFields::Github {
                owner_repo: "owner/repo".into(),
                path_in_repo: "skills/test.md".into(),
                ref_: "main".into(),
            },
        };
        let patch_text = skillfile_core::patch::generate_patch(
            "# Test\n\nOriginal.\n",
            "# Test\n\nModified.\n",
            "test.md",
        );
        skillfile_core::patch::write_patch(&entry, &patch_text, dir.path()).unwrap();

        let target = make_target("claude-code", Scope::Local);
        install_entry(&entry, &target, dir.path(), None).unwrap();

        let dest = dir.path().join(".claude/skills/test.md");
        assert_eq!(
            std::fs::read_to_string(&dest).unwrap(),
            "# Test\n\nModified.\n"
        );
    }

    #[test]
    fn install_patch_conflict_returns_error() {
        let dir = tempfile::tempdir().unwrap();

        let vdir = dir.path().join(".skillfile/cache/skills/test");
        std::fs::create_dir_all(&vdir).unwrap();
        // Cache has completely different content from what the patch expects
        std::fs::write(vdir.join("test.md"), "totally different\ncontent\n").unwrap();

        let entry = Entry {
            entity_type: EntityType::Skill,
            name: "test".into(),
            source: SourceFields::Github {
                owner_repo: "owner/repo".into(),
                path_in_repo: "skills/test.md".into(),
                ref_: "main".into(),
            },
        };
        // Write a patch that expects a line that doesn't exist
        let bad_patch =
            "--- a/test.md\n+++ b/test.md\n@@ -1 +1 @@\n-expected_original_line\n+modified\n";
        skillfile_core::patch::write_patch(&entry, bad_patch, dir.path()).unwrap();

        // Deploy the entry
        let installed_dir = dir.path().join(".claude/skills");
        std::fs::create_dir_all(&installed_dir).unwrap();
        std::fs::write(
            installed_dir.join("test.md"),
            "totally different\ncontent\n",
        )
        .unwrap();

        let target = make_target("claude-code", Scope::Local);
        let result = install_entry(&entry, &target, dir.path(), None);
        assert!(result.is_err());
        // Should be a PatchConflict error
        matches!(result.unwrap_err(), SkillfileError::PatchConflict { .. });
    }

    // -- Multi-adapter --

    #[test]
    fn install_local_skill_gemini_cli() {
        let dir = tempfile::tempdir().unwrap();
        let source_file = dir.path().join("skills/my-skill.md");
        std::fs::create_dir_all(source_file.parent().unwrap()).unwrap();
        std::fs::write(&source_file, "# My Skill").unwrap();

        let entry = make_local_entry("my-skill", "skills/my-skill.md");
        let target = make_target("gemini-cli", Scope::Local);
        install_entry(&entry, &target, dir.path(), None).unwrap();

        let dest = dir.path().join(".gemini/skills/my-skill.md");
        assert!(dest.exists());
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "# My Skill");
    }

    #[test]
    fn install_local_skill_codex() {
        let dir = tempfile::tempdir().unwrap();
        let source_file = dir.path().join("skills/my-skill.md");
        std::fs::create_dir_all(source_file.parent().unwrap()).unwrap();
        std::fs::write(&source_file, "# My Skill").unwrap();

        let entry = make_local_entry("my-skill", "skills/my-skill.md");
        let target = make_target("codex", Scope::Local);
        install_entry(&entry, &target, dir.path(), None).unwrap();

        let dest = dir.path().join(".codex/skills/my-skill.md");
        assert!(dest.exists());
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "# My Skill");
    }

    #[test]
    fn codex_skips_agent_entries() {
        let dir = tempfile::tempdir().unwrap();
        let entry = make_agent_entry("my-agent");
        let target = make_target("codex", Scope::Local);
        install_entry(&entry, &target, dir.path(), None).unwrap();

        assert!(!dir.path().join(".codex").exists());
    }

    #[test]
    fn install_github_agent_gemini_cli() {
        let dir = tempfile::tempdir().unwrap();
        let vdir = dir.path().join(".skillfile/cache/agents/my-agent");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("agent.md"), "# Agent").unwrap();

        let entry = make_agent_entry("my-agent");
        let target = make_target("gemini-cli", Scope::Local);
        install_entry(
            &entry,
            &target,
            dir.path(),
            Some(&InstallOptions::default()),
        )
        .unwrap();

        let dest = dir.path().join(".gemini/agents/my-agent.md");
        assert!(dest.exists());
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "# Agent");
    }

    #[test]
    fn install_skill_multi_adapter() {
        for adapter in &["claude-code", "gemini-cli", "codex"] {
            let dir = tempfile::tempdir().unwrap();
            let source_file = dir.path().join("skills/my-skill.md");
            std::fs::create_dir_all(source_file.parent().unwrap()).unwrap();
            std::fs::write(&source_file, "# Multi Skill").unwrap();

            let entry = make_local_entry("my-skill", "skills/my-skill.md");
            let target = make_target(adapter, Scope::Local);
            install_entry(&entry, &target, dir.path(), None).unwrap();

            let prefix = match *adapter {
                "claude-code" => ".claude",
                "gemini-cli" => ".gemini",
                "codex" => ".codex",
                _ => unreachable!(),
            };
            let dest = dir.path().join(format!("{prefix}/skills/my-skill.md"));
            assert!(dest.exists(), "Failed for adapter {adapter}");
            assert_eq!(std::fs::read_to_string(&dest).unwrap(), "# Multi Skill");
        }
    }

    // -- cmd_install --

    #[test]
    fn cmd_install_no_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let result = cmd_install(dir.path(), false, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn cmd_install_no_install_targets() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Skillfile"),
            "local  skill  foo  skills/foo.md\n",
        )
        .unwrap();

        let result = cmd_install(dir.path(), false, false);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("No install targets"));
    }

    #[test]
    fn cmd_install_dry_run_no_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Skillfile"),
            "install  claude-code  local\nlocal  skill  foo  skills/foo.md\n",
        )
        .unwrap();
        let source_file = dir.path().join("skills/foo.md");
        std::fs::create_dir_all(source_file.parent().unwrap()).unwrap();
        std::fs::write(&source_file, "# Foo").unwrap();

        cmd_install(dir.path(), true, false).unwrap();

        assert!(!dir.path().join(".claude").exists());
    }

    #[test]
    fn cmd_install_deploys_to_multiple_adapters() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Skillfile"),
            "install  claude-code  local\n\
             install  gemini-cli  local\n\
             install  codex  local\n\
             local  skill  foo  skills/foo.md\n\
             local  agent  bar  agents/bar.md\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("skills")).unwrap();
        std::fs::write(dir.path().join("skills/foo.md"), "# Foo").unwrap();
        std::fs::create_dir_all(dir.path().join("agents")).unwrap();
        std::fs::write(dir.path().join("agents/bar.md"), "# Bar").unwrap();

        cmd_install(dir.path(), false, false).unwrap();

        // skill deployed to all three adapters
        assert!(dir.path().join(".claude/skills/foo.md").exists());
        assert!(dir.path().join(".gemini/skills/foo.md").exists());
        assert!(dir.path().join(".codex/skills/foo.md").exists());

        // agent deployed to claude-code and gemini-cli but NOT codex
        assert!(dir.path().join(".claude/agents/bar.md").exists());
        assert!(dir.path().join(".gemini/agents/bar.md").exists());
        assert!(!dir.path().join(".codex/agents").exists());
    }

    #[test]
    fn cmd_install_pending_conflict_blocks() {
        use skillfile_core::conflict::write_conflict;
        use skillfile_core::models::ConflictState;

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Skillfile"),
            "install  claude-code  local\nlocal  skill  foo  skills/foo.md\n",
        )
        .unwrap();

        write_conflict(
            dir.path(),
            &ConflictState {
                entry: "foo".into(),
                entity_type: "skill".into(),
                old_sha: "aaa".into(),
                new_sha: "bbb".into(),
            },
        )
        .unwrap();

        let result = cmd_install(dir.path(), false, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("pending conflict"));
    }
}
