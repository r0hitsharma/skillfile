use std::path::{Path, PathBuf};

use crate::error::SkillfileError;
use crate::models::Entry;

pub const PATCHES_DIR: &str = ".skillfile/patches";

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

pub fn patches_root(repo_root: &Path) -> PathBuf {
    repo_root.join(PATCHES_DIR)
}

/// Path to the patch file for a single-file entry.
/// e.g. `.skillfile/patches/agents/my-agent.patch`
pub fn patch_path(entry: &Entry, repo_root: &Path) -> PathBuf {
    patches_root(repo_root)
        .join(format!("{}s", entry.entity_type))
        .join(format!("{}.patch", entry.name))
}

pub fn has_patch(entry: &Entry, repo_root: &Path) -> bool {
    patch_path(entry, repo_root).exists()
}

pub fn write_patch(
    entry: &Entry,
    patch_text: &str,
    repo_root: &Path,
) -> Result<(), SkillfileError> {
    let p = patch_path(entry, repo_root);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&p, patch_text)?;
    Ok(())
}

pub fn read_patch(entry: &Entry, repo_root: &Path) -> Result<String, SkillfileError> {
    let p = patch_path(entry, repo_root);
    Ok(std::fs::read_to_string(&p)?)
}

pub fn remove_patch(entry: &Entry, repo_root: &Path) -> Result<(), SkillfileError> {
    let p = patch_path(entry, repo_root);
    if !p.exists() {
        return Ok(());
    }
    std::fs::remove_file(&p)?;
    if let Some(parent) = p.parent() {
        if parent.exists() {
            let is_empty = std::fs::read_dir(parent)?.next().is_none();
            if is_empty {
                let _ = std::fs::remove_dir(parent);
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Directory entry patches (one .patch file per modified file)
// ---------------------------------------------------------------------------

/// Path to a per-file patch within a directory entry.
/// e.g. `.skillfile/patches/skills/architecture-patterns/SKILL.md.patch`
pub fn dir_patch_path(entry: &Entry, filename: &str, repo_root: &Path) -> PathBuf {
    patches_root(repo_root)
        .join(format!("{}s", entry.entity_type))
        .join(&entry.name)
        .join(format!("{filename}.patch"))
}

pub fn has_dir_patch(entry: &Entry, repo_root: &Path) -> bool {
    let d = patches_root(repo_root)
        .join(format!("{}s", entry.entity_type))
        .join(&entry.name);
    if !d.is_dir() {
        return false;
    }
    walkdir(&d)
        .into_iter()
        .any(|p| p.extension().is_some_and(|e| e == "patch"))
}

pub fn write_dir_patch(
    entry: &Entry,
    filename: &str,
    patch_text: &str,
    repo_root: &Path,
) -> Result<(), SkillfileError> {
    let p = dir_patch_path(entry, filename, repo_root);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&p, patch_text)?;
    Ok(())
}

pub fn remove_dir_patch(
    entry: &Entry,
    filename: &str,
    repo_root: &Path,
) -> Result<(), SkillfileError> {
    let p = dir_patch_path(entry, filename, repo_root);
    if !p.exists() {
        return Ok(());
    }
    std::fs::remove_file(&p)?;
    if let Some(parent) = p.parent() {
        if parent.exists() {
            let is_empty = std::fs::read_dir(parent)?.next().is_none();
            if is_empty {
                let _ = std::fs::remove_dir(parent);
            }
        }
    }
    Ok(())
}

pub fn remove_all_dir_patches(entry: &Entry, repo_root: &Path) -> Result<(), SkillfileError> {
    let d = patches_root(repo_root)
        .join(format!("{}s", entry.entity_type))
        .join(&entry.name);
    if d.is_dir() {
        std::fs::remove_dir_all(&d)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Diff generation
// ---------------------------------------------------------------------------

/// Generate a unified diff of original → modified. Empty string if identical.
/// All output lines are guaranteed to end with '\n'.
/// Format: `--- a/{label}` / `+++ b/{label}`, 3 lines of context.
pub fn generate_patch(original: &str, modified: &str, label: &str) -> String {
    if original == modified {
        return String::new();
    }

    let diff = similar::TextDiff::from_lines(original, modified);
    let raw = format!(
        "{}",
        diff.unified_diff()
            .context_radius(3)
            .header(&format!("a/{label}"), &format!("b/{label}"))
    );

    if raw.is_empty() {
        return String::new();
    }

    // Post-process: remove "\ No newline at end of file" markers,
    // normalize any lines not ending with \n.
    let mut result = String::new();
    for line in raw.split_inclusive('\n') {
        if line.starts_with("\\ ") {
            // "\ No newline at end of file" — normalize preceding line
            if !result.ends_with('\n') {
                result.push('\n');
            }
            // Skip the marker
        } else if line.ends_with('\n') {
            result.push_str(line);
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Patch application (pure Rust, no subprocess)
// ---------------------------------------------------------------------------

struct Hunk {
    orig_start: usize, // 1-based line number from @@ header
    body: Vec<String>,
}

fn parse_hunks(patch_text: &str) -> Result<Vec<Hunk>, SkillfileError> {
    let lines: Vec<&str> = patch_text.split_inclusive('\n').collect();
    let mut pi = 0;

    // Skip file headers (--- / +++ lines)
    while pi < lines.len() && (lines[pi].starts_with("--- ") || lines[pi].starts_with("+++ ")) {
        pi += 1;
    }

    let mut hunks: Vec<Hunk> = Vec::new();

    while pi < lines.len() {
        let pl = lines[pi];
        if !pl.starts_with("@@ ") {
            pi += 1;
            continue;
        }

        // Parse hunk header: @@ -l[,s] +l[,s] @@
        // We only need orig_start (the -l part)
        let orig_start = pl
            .split_whitespace()
            .nth(1) // "-l[,s]"
            .and_then(|s| s.trim_start_matches('-').split(',').next())
            .and_then(|n| n.parse::<usize>().ok())
            .ok_or_else(|| SkillfileError::Manifest(format!("malformed hunk header: {pl:?}")))?;

        pi += 1;
        let mut body: Vec<String> = Vec::new();

        while pi < lines.len() {
            let hl = lines[pi];
            if hl.starts_with("@@ ") || hl.starts_with("--- ") || hl.starts_with("+++ ") {
                break;
            }
            if hl.starts_with("\\ ") {
                // "\ No newline at end of file" — skip
                pi += 1;
                continue;
            }
            body.push(hl.to_string());
            pi += 1;
        }

        hunks.push(Hunk { orig_start, body });
    }

    Ok(hunks)
}

fn try_hunk_at(lines: &[String], start: usize, ctx_lines: &[&str]) -> bool {
    if start + ctx_lines.len() > lines.len() {
        return false;
    }
    for (i, expected) in ctx_lines.iter().enumerate() {
        if lines[start + i].trim_end_matches('\n') != *expected {
            return false;
        }
    }
    true
}

fn find_hunk_position(
    lines: &[String],
    hunk_start: usize,
    ctx_lines: &[&str],
    min_pos: usize,
) -> Result<usize, SkillfileError> {
    if try_hunk_at(lines, hunk_start, ctx_lines) {
        return Ok(hunk_start);
    }

    for delta in 1..100usize {
        let candidates = [Some(hunk_start + delta), hunk_start.checked_sub(delta)];
        for candidate in candidates.into_iter().flatten() {
            if candidate < min_pos || candidate > lines.len() {
                continue;
            }
            if try_hunk_at(lines, candidate, ctx_lines) {
                return Ok(candidate);
            }
        }
    }

    if !ctx_lines.is_empty() {
        return Err(SkillfileError::Manifest(format!(
            "context mismatch: cannot find context starting with {:?} near line {}",
            ctx_lines[0],
            hunk_start + 1
        )));
    }
    Err(SkillfileError::Manifest(
        "patch extends beyond end of file".into(),
    ))
}

/// Apply a unified diff to original text, returning modified content.
/// Pure implementation — no subprocess, no `patch` binary required.
/// Only handles patches produced by `generate_patch()` (unified diff format).
/// Returns an error if the patch does not apply cleanly.
pub fn apply_patch_pure(original: &str, patch_text: &str) -> Result<String, SkillfileError> {
    if patch_text.is_empty() {
        return Ok(original.to_string());
    }

    // Split into lines preserving newlines (like Python's splitlines(keepends=True))
    let lines: Vec<String> = original
        .split_inclusive('\n')
        .map(|s| s.to_string())
        .collect();

    let mut output: Vec<String> = Vec::new();
    let mut li = 0usize; // current position in lines (0-based)

    for hunk in parse_hunks(patch_text)? {
        // Build context: lines with ' ' or '-' prefix, stripped of prefix and trailing \n
        let ctx_lines: Vec<&str> = hunk
            .body
            .iter()
            .filter(|hl| !hl.is_empty() && (hl.starts_with(' ') || hl.starts_with('-')))
            .map(|hl| hl[1..].trim_end_matches('\n'))
            .collect();

        let hunk_start =
            find_hunk_position(&lines, hunk.orig_start.saturating_sub(1), &ctx_lines, li)?;

        // Copy unchanged lines before this hunk
        output.extend_from_slice(&lines[li..hunk_start]);
        li = hunk_start;

        // Apply hunk: emit context and additions, skip removals
        for hl in &hunk.body {
            if hl.is_empty() {
                continue;
            }
            match hl.chars().next() {
                Some(' ') => {
                    if li < lines.len() {
                        output.push(lines[li].clone());
                        li += 1;
                    }
                }
                Some('-') => {
                    li += 1;
                }
                Some('+') => {
                    output.push(hl[1..].to_string());
                }
                _ => {}
            }
        }
    }

    // Copy remaining lines
    output.extend_from_slice(&lines[li..]);
    Ok(output.concat())
}

// ---------------------------------------------------------------------------
// Directory walking helper
// ---------------------------------------------------------------------------

pub fn walkdir(dir: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    walkdir_inner(dir, &mut result);
    result.sort();
    result
}

fn walkdir_inner(dir: &Path, result: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walkdir_inner(&path, result);
            } else {
                result.push(path);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SourceFields;

    fn github_entry(name: &str, entity_type: &str) -> Entry {
        Entry {
            entity_type: entity_type.to_string(),
            name: name.to_string(),
            source: SourceFields::Github {
                owner_repo: "owner/repo".into(),
                path_in_repo: "agents/test.md".into(),
                ref_: "main".into(),
            },
        }
    }

    // --- generate_patch ---

    #[test]
    fn generate_patch_identical_returns_empty() {
        assert_eq!(generate_patch("hello\n", "hello\n", "test.md"), "");
    }

    #[test]
    fn generate_patch_has_headers() {
        let p = generate_patch("old\n", "new\n", "test.md");
        assert!(p.contains("--- a/test.md"), "missing fromfile header");
        assert!(p.contains("+++ b/test.md"), "missing tofile header");
    }

    #[test]
    fn generate_patch_add_line() {
        let p = generate_patch("line1\n", "line1\nline2\n", "test.md");
        assert!(p.contains("+line2"));
    }

    #[test]
    fn generate_patch_remove_line() {
        let p = generate_patch("line1\nline2\n", "line1\n", "test.md");
        assert!(p.contains("-line2"));
    }

    #[test]
    fn generate_patch_all_lines_end_with_newline() {
        let p = generate_patch("a\nb\n", "a\nc\n", "test.md");
        for seg in p.split_inclusive('\n') {
            assert!(seg.ends_with('\n'), "line does not end with \\n: {seg:?}");
        }
    }

    // --- apply_patch_pure ---

    #[test]
    fn apply_patch_empty_patch_returns_original() {
        let result = apply_patch_pure("hello\n", "").unwrap();
        assert_eq!(result, "hello\n");
    }

    #[test]
    fn apply_patch_round_trip_add_line() {
        let orig = "line1\nline2\n";
        let modified = "line1\nline2\nline3\n";
        let patch = generate_patch(orig, modified, "test.md");
        let result = apply_patch_pure(orig, &patch).unwrap();
        assert_eq!(result, modified);
    }

    #[test]
    fn apply_patch_round_trip_remove_line() {
        let orig = "line1\nline2\nline3\n";
        let modified = "line1\nline3\n";
        let patch = generate_patch(orig, modified, "test.md");
        let result = apply_patch_pure(orig, &patch).unwrap();
        assert_eq!(result, modified);
    }

    #[test]
    fn apply_patch_round_trip_modify_line() {
        let orig = "# Title\n\nSome text here.\n";
        let modified = "# Title\n\nSome modified text here.\n";
        let patch = generate_patch(orig, modified, "test.md");
        let result = apply_patch_pure(orig, &patch).unwrap();
        assert_eq!(result, modified);
    }

    #[test]
    fn apply_patch_multi_hunk() {
        let orig = (0..20).map(|i| format!("line{i}\n")).collect::<String>();
        let mut modified = orig.clone();
        modified = modified.replace("line2\n", "MODIFIED2\n");
        modified = modified.replace("line15\n", "MODIFIED15\n");
        let patch = generate_patch(&orig, &modified, "test.md");
        assert!(patch.contains("@@"), "should have hunk headers");
        let result = apply_patch_pure(&orig, &patch).unwrap();
        assert_eq!(result, modified);
    }

    #[test]
    fn apply_patch_context_mismatch_errors() {
        let orig = "line1\nline2\n";
        let patch = "--- a/test.md\n+++ b/test.md\n@@ -1,2 +1,2 @@\n-totally_wrong\n+new\n";
        let result = apply_patch_pure(orig, patch);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("context mismatch"));
    }

    // --- Patch path helpers ---

    #[test]
    fn patch_path_single_file_agent() {
        let entry = github_entry("my-agent", "agent");
        let root = Path::new("/repo");
        let p = patch_path(&entry, root);
        assert_eq!(
            p,
            Path::new("/repo/.skillfile/patches/agents/my-agent.patch")
        );
    }

    #[test]
    fn patch_path_single_file_skill() {
        let entry = github_entry("my-skill", "skill");
        let root = Path::new("/repo");
        let p = patch_path(&entry, root);
        assert_eq!(
            p,
            Path::new("/repo/.skillfile/patches/skills/my-skill.patch")
        );
    }

    #[test]
    fn dir_patch_path_returns_correct() {
        let entry = github_entry("lang-pro", "skill");
        let root = Path::new("/repo");
        let p = dir_patch_path(&entry, "python.md", root);
        assert_eq!(
            p,
            Path::new("/repo/.skillfile/patches/skills/lang-pro/python.md.patch")
        );
    }

    #[test]
    fn write_read_remove_patch_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let entry = github_entry("test-agent", "agent");
        let patch_text = "--- a/test-agent.md\n+++ b/test-agent.md\n@@ -1 +1 @@\n-old\n+new\n";
        write_patch(&entry, patch_text, dir.path()).unwrap();
        assert!(has_patch(&entry, dir.path()));
        let read = read_patch(&entry, dir.path()).unwrap();
        assert_eq!(read, patch_text);
        remove_patch(&entry, dir.path()).unwrap();
        assert!(!has_patch(&entry, dir.path()));
    }

    #[test]
    fn has_dir_patch_detects_patches() {
        let dir = tempfile::tempdir().unwrap();
        let entry = github_entry("lang-pro", "skill");
        assert!(!has_dir_patch(&entry, dir.path()));
        write_dir_patch(&entry, "python.md", "patch content", dir.path()).unwrap();
        assert!(has_dir_patch(&entry, dir.path()));
    }

    #[test]
    fn remove_all_dir_patches_clears_dir() {
        let dir = tempfile::tempdir().unwrap();
        let entry = github_entry("lang-pro", "skill");
        write_dir_patch(&entry, "python.md", "p1", dir.path()).unwrap();
        write_dir_patch(&entry, "typescript.md", "p2", dir.path()).unwrap();
        assert!(has_dir_patch(&entry, dir.path()));
        remove_all_dir_patches(&entry, dir.path()).unwrap();
        assert!(!has_dir_patch(&entry, dir.path()));
    }
}
