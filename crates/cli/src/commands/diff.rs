use std::io::Write as IoWrite;
use std::path::Path;

use skillfile_core::conflict::read_conflict;
use skillfile_core::error::SkillfileError;
use skillfile_core::lock::{lock_key, read_lock};
use skillfile_core::models::Entry;
use skillfile_core::parser::{find_entry_in, parse_manifest, MANIFEST_NAME};
use skillfile_deploy::paths::{installed_dir_files, installed_path};
use skillfile_sources::strategy::{content_file, is_dir_entry};
use skillfile_sources::sync::vendor_dir_for;

use crate::patch::walkdir;

fn diff_local_single(entry: &Entry, sha: &str, repo_root: &Path) -> Result<(), SkillfileError> {
    let result = parse_manifest(&repo_root.join(MANIFEST_NAME))?;
    let vdir = vendor_dir_for(entry, repo_root);
    let cf = content_file(entry);
    if cf.is_empty() {
        return Err(SkillfileError::Manifest(format!(
            "'{}' is not cached — run `skillfile install` first",
            entry.name
        )));
    }
    let cache_file = vdir.join(&cf);
    if !cache_file.exists() {
        return Err(SkillfileError::Manifest(format!(
            "'{}' is not cached — run `skillfile install` first",
            entry.name
        )));
    }

    let dest = installed_path(entry, &result.manifest, repo_root)?;
    if !dest.exists() {
        return Err(SkillfileError::Manifest(format!(
            "'{}' is not installed — run `skillfile install` first",
            entry.name
        )));
    }

    let upstream = std::fs::read_to_string(&cache_file)?;
    let installed_text = std::fs::read_to_string(&dest)?;

    let diff_text = similar::TextDiff::from_lines(upstream.as_str(), installed_text.as_str());
    let formatted = format!(
        "{}",
        diff_text.unified_diff().context_radius(3).header(
            &format!(
                "a/{}.md (upstream sha={})",
                entry.name,
                &sha[..sha.len().min(12)]
            ),
            &format!("b/{}.md (installed)", entry.name),
        )
    );

    if formatted.is_empty() {
        println!("'{}' is clean — no local modifications", entry.name);
    } else {
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        out.write_all(formatted.as_bytes())?;
    }

    Ok(())
}

fn diff_local_dir(entry: &Entry, sha: &str, repo_root: &Path) -> Result<(), SkillfileError> {
    let result = parse_manifest(&repo_root.join(MANIFEST_NAME))?;
    let vdir = vendor_dir_for(entry, repo_root);
    if !vdir.is_dir() {
        return Err(SkillfileError::Manifest(format!(
            "'{}' is not cached — run `skillfile install` first",
            entry.name
        )));
    }

    let installed = installed_dir_files(entry, &result.manifest, repo_root)?;
    if installed.is_empty() {
        return Err(SkillfileError::Manifest(format!(
            "'{}' is not installed — run `skillfile install` first",
            entry.name
        )));
    }

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut any_diff = false;

    for cache_file in walkdir(&vdir) {
        if cache_file.file_name().is_some_and(|n| n == ".meta") {
            continue;
        }
        let filename = match cache_file.strip_prefix(&vdir).ok().and_then(|p| p.to_str()) {
            Some(f) => f.to_string(),
            None => continue,
        };
        let inst_path = match installed.get(&filename) {
            Some(p) => p,
            None => continue,
        };
        if !inst_path.exists() {
            continue;
        }

        let original_text = std::fs::read_to_string(&cache_file)?;
        let installed_text = std::fs::read_to_string(inst_path)?;
        let diff_text =
            similar::TextDiff::from_lines(original_text.as_str(), installed_text.as_str());
        let formatted = format!(
            "{}",
            diff_text.unified_diff().context_radius(3).header(
                &format!(
                    "a/{}/{filename} (upstream sha={})",
                    entry.name,
                    &sha[..sha.len().min(12)]
                ),
                &format!("b/{}/{filename} (installed)", entry.name),
            )
        );

        if !formatted.is_empty() {
            any_diff = true;
            out.write_all(formatted.as_bytes())?;
        }
    }

    if !any_diff {
        println!("'{}' is clean — no local modifications", entry.name);
    }

    Ok(())
}

pub fn cmd_diff(name: &str, repo_root: &Path) -> Result<(), SkillfileError> {
    let manifest_path = repo_root.join(MANIFEST_NAME);
    let result = parse_manifest(&manifest_path)?;
    let entry = find_entry_in(name, &result.manifest)?;

    // Check if there's a pending conflict for this entry
    let conflict = read_conflict(repo_root)?;
    if let Some(ref c) = conflict {
        if c.entry == name {
            return diff_conflict(entry, c, repo_root);
        }
    }

    if entry.source_type() == "local" {
        println!("'{name}' is a local entry — nothing to diff");
        return Ok(());
    }

    let locked = read_lock(repo_root)?;
    let key = lock_key(entry);
    if !locked.contains_key(&key) {
        return Err(SkillfileError::Manifest(format!(
            "'{name}' is not locked — run `skillfile install` first"
        )));
    }
    let sha = locked[&key].sha.clone();

    if is_dir_entry(entry) {
        diff_local_dir(entry, &sha, repo_root)
    } else {
        diff_local_single(entry, &sha, repo_root)
    }
}

fn diff_conflict(
    entry: &Entry,
    conflict: &skillfile_core::models::ConflictState,
    _repo_root: &Path,
) -> Result<(), SkillfileError> {
    // Conflict mode: fetch old and new upstream, show upstream delta
    // This requires network access
    eprintln!(
        "  fetching upstream at old sha={} ...",
        &conflict.old_sha[..conflict.old_sha.len().min(12)]
    );
    let client = skillfile_sources::http::UreqClient::new();

    if is_dir_entry(entry) {
        diff_conflict_dir(entry, conflict, &client)?;
    } else {
        diff_conflict_single(entry, conflict, &client)?;
    }
    Ok(())
}

fn diff_conflict_single(
    entry: &Entry,
    conflict: &skillfile_core::models::ConflictState,
    client: &dyn skillfile_sources::http::HttpClient,
) -> Result<(), SkillfileError> {
    let old_content = skillfile_sources::sync::fetch_file_at_sha(client, entry, &conflict.old_sha)?;
    eprintln!("done");
    eprintln!(
        "  fetching upstream at new sha={} ...",
        &conflict.new_sha[..conflict.new_sha.len().min(12)]
    );
    let new_content = skillfile_sources::sync::fetch_file_at_sha(client, entry, &conflict.new_sha)?;
    eprintln!("done\n");

    let diff_text = similar::TextDiff::from_lines(old_content.as_str(), new_content.as_str());
    let formatted = format!(
        "{}",
        diff_text.unified_diff().context_radius(3).header(
            &format!(
                "{}.md (old upstream sha={})",
                entry.name,
                &conflict.old_sha[..conflict.old_sha.len().min(12)]
            ),
            &format!(
                "{}.md (new upstream sha={})",
                entry.name,
                &conflict.new_sha[..conflict.new_sha.len().min(12)]
            ),
        )
    );

    if formatted.is_empty() {
        println!("No upstream changes detected (patch conflict may be due to local file drift).");
    } else {
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        out.write_all(formatted.as_bytes())?;
    }
    Ok(())
}

fn diff_conflict_dir(
    entry: &Entry,
    conflict: &skillfile_core::models::ConflictState,
    client: &dyn skillfile_sources::http::HttpClient,
) -> Result<(), SkillfileError> {
    let old_files = skillfile_sources::sync::fetch_dir_at_sha(client, entry, &conflict.old_sha)?;
    eprintln!("done");
    eprintln!(
        "  fetching upstream at new sha={} ...",
        &conflict.new_sha[..conflict.new_sha.len().min(12)]
    );
    let new_files = skillfile_sources::sync::fetch_dir_at_sha(client, entry, &conflict.new_sha)?;
    eprintln!("done\n");

    let mut all_filenames: Vec<String> = old_files
        .keys()
        .chain(new_files.keys())
        .cloned()
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    all_filenames.sort();

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut any_diff = false;

    for filename in &all_filenames {
        let old_content = old_files.get(filename).map(|s| s.as_str()).unwrap_or("");
        let new_content = new_files.get(filename).map(|s| s.as_str()).unwrap_or("");
        let diff_text = similar::TextDiff::from_lines(old_content, new_content);
        let formatted = format!(
            "{}",
            diff_text.unified_diff().context_radius(3).header(
                &format!(
                    "{}/{filename} (old upstream sha={})",
                    entry.name,
                    &conflict.old_sha[..conflict.old_sha.len().min(12)]
                ),
                &format!(
                    "{}/{filename} (new upstream sha={})",
                    entry.name,
                    &conflict.new_sha[..conflict.new_sha.len().min(12)]
                ),
            )
        );
        if !formatted.is_empty() {
            any_diff = true;
            out.write_all(formatted.as_bytes())?;
        }
    }

    if !any_diff {
        println!("No upstream changes detected (patch conflict may be due to local file drift).");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_manifest(dir: &Path, content: &str) {
        std::fs::write(dir.join(MANIFEST_NAME), content).unwrap();
    }

    fn write_lock_file(dir: &Path, content: &str) {
        std::fs::write(dir.join("Skillfile.lock"), content).unwrap();
    }

    fn make_lock_json(name: &str, entity_type: &str) -> String {
        format!(
            r#"{{
  "github/{entity_type}/{name}": {{
    "sha": "abc123def456abcdef",
    "raw_url": "https://raw.githubusercontent.com/owner/repo/abc123/test.md"
  }}
}}"#
        )
    }

    #[test]
    fn diff_no_manifest_errors() {
        let dir = tempfile::tempdir().unwrap();
        let result = cmd_diff("foo", dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn diff_local_entry_prints_message() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(dir.path(), "local  skill  skills/foo.md\n");
        // cmd_diff will print "local entry — nothing to diff"
        // Since it goes to stdout which we can't capture in unit tests easily,
        // just verify it doesn't error
        // The cmd_diff function looks up the entry in the manifest
        // We need the manifest to have the "foo" entry with local source
        let result = cmd_diff("foo", dir.path());
        // "foo" is not in the manifest (it's inferred as "foo" from "skills/foo.md")
        assert!(result.is_ok());
    }

    #[test]
    fn diff_not_locked_errors() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(dir.path(), "github  skill  owner/repo  skills/test.md\n");
        write_lock_file(dir.path(), "{}");
        let result = cmd_diff("test", dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not locked"));
    }

    #[test]
    fn diff_not_cached_errors() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "install  claude-code  local\ngithub  skill  owner/repo  skills/test.md\n",
        );
        write_lock_file(dir.path(), &make_lock_json("test", "skill"));
        // no cache files
        let result = cmd_diff("test", dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not cached"));
    }

    #[test]
    fn diff_not_installed_errors() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "install  claude-code  local\ngithub  skill  owner/repo  skills/test.md\n",
        );
        write_lock_file(dir.path(), &make_lock_json("test", "skill"));

        // Create cache but not installed
        let vdir = dir.path().join(".skillfile/cache/skills/test");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("test.md"), "content\n").unwrap();

        let result = cmd_diff("test", dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not installed"));
    }

    #[test]
    fn diff_clean_shows_clean() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "install  claude-code  local\ngithub  skill  owner/repo  skills/test.md\n",
        );
        write_lock_file(dir.path(), &make_lock_json("test", "skill"));

        let content = "# Test\n\nContent.\n";
        let vdir = dir.path().join(".skillfile/cache/skills/test");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("test.md"), content).unwrap();

        let installed_dir = dir.path().join(".claude/skills");
        std::fs::create_dir_all(&installed_dir).unwrap();
        std::fs::write(installed_dir.join("test.md"), content).unwrap();

        // Should succeed (prints "is clean")
        let result = cmd_diff("test", dir.path());
        assert!(result.is_ok());
    }

    #[test]
    fn diff_modified_produces_output() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "install  claude-code  local\ngithub  skill  owner/repo  skills/test.md\n",
        );
        write_lock_file(dir.path(), &make_lock_json("test", "skill"));

        let vdir = dir.path().join(".skillfile/cache/skills/test");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("test.md"), "original\n").unwrap();

        let installed_dir = dir.path().join(".claude/skills");
        std::fs::create_dir_all(&installed_dir).unwrap();
        std::fs::write(installed_dir.join("test.md"), "modified\n").unwrap();

        // Should succeed (diff goes to stdout)
        let result = cmd_diff("test", dir.path());
        assert!(result.is_ok());
    }
}
