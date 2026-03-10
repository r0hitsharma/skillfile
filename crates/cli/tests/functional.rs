/// Functional tests: invoke the compiled `skillfile` binary against the real GitHub API.
///
/// These tests require a GitHub token and network access.
/// Set GITHUB_TOKEN or GH_TOKEN, or have `gh auth login` configured.
/// They fail hard (not skip) when no token is available — same as the Python functional tests.
///
/// The test Skillfile uses the same public repos as the Python functional tests.
use std::path::Path;

use assert_cmd::cargo_bin_cmd;
use predicates::prelude::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const TEST_SKILLFILE: &str = "\
install  claude-code  local\n\
\n\
# Single-file agent\n\
github  agent  code-refactorer  iannuttall/claude-agents  agents/code-refactorer.md\n\
\n\
# Single-file skill\n\
github  skill  requesting-code-review  obra/superpowers  skills/requesting-code-review\n\
";

/// Create a temp dir with the test Skillfile.
fn make_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Skillfile"), TEST_SKILLFILE).unwrap();
    dir
}

/// Check whether a GitHub token is available via environment variable.
fn has_github_token() -> bool {
    std::env::var("GITHUB_TOKEN").is_ok() || std::env::var("GH_TOKEN").is_ok()
}

/// Panic if no GitHub token is available. Functional tests fail hard, not skip.
fn require_github_token() {
    assert!(
        has_github_token(),
        "GitHub token required for functional tests. \
         Set GITHUB_TOKEN or GH_TOKEN, or run `gh auth login`."
    );
}

/// Run `skillfile <args>` in `dir` and return the Command (pre-configured).
fn sf(dir: &Path) -> assert_cmd::Command {
    let mut cmd = cargo_bin_cmd!("skillfile");
    cmd.current_dir(dir);
    cmd
}

// ---------------------------------------------------------------------------
// Tests that do NOT require network access
// ---------------------------------------------------------------------------

#[test]
fn validate_golden_path() {
    let dir = make_repo();
    sf(dir.path())
        .arg("validate")
        .assert()
        .success()
        .stderr(predicate::str::contains("error").not())
        .stdout(predicate::str::contains("error").not());
}

#[test]
fn sort_golden_path() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Skillfile"),
        "install  claude-code  local\n\
         github  skill  zebra  owner/repo  skills/z.md\n\
         github  skill  alpha  owner/repo  skills/a.md\n",
    )
    .unwrap();

    sf(dir.path()).arg("sort").assert().success();

    let text = std::fs::read_to_string(dir.path().join("Skillfile")).unwrap();
    let entry_lines: Vec<&str> = text.lines().filter(|l| l.starts_with("github")).collect();
    assert!(entry_lines[0].contains("alpha"), "alpha should be first");
    assert!(entry_lines[1].contains("zebra"), "zebra should be second");
}

#[test]
fn add_then_remove() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Skillfile"), "# empty\n").unwrap();

    // Add a new entry (no install targets, so no network call)
    sf(dir.path())
        .args([
            "add",
            "github",
            "skill",
            "my-new-skill",
            "owner/repo",
            "skills/test.md",
        ])
        .assert()
        .success();

    let sf_text = std::fs::read_to_string(dir.path().join("Skillfile")).unwrap();
    assert!(
        sf_text.contains("my-new-skill"),
        "entry should be in Skillfile"
    );

    // Remove it
    sf(dir.path())
        .args(["remove", "my-new-skill"])
        .assert()
        .success();

    let sf_text = std::fs::read_to_string(dir.path().join("Skillfile")).unwrap();
    assert!(!sf_text.contains("my-new-skill"), "entry should be removed");
}

// ---------------------------------------------------------------------------
// Tests that require network access (fail hard without token)
// ---------------------------------------------------------------------------

#[test]
fn sync_golden_path() {
    require_github_token();
    let dir = make_repo();

    sf(dir.path()).arg("sync").assert().success();

    // Lock file written
    assert!(dir.path().join("Skillfile.lock").exists());
    let lock_text = std::fs::read_to_string(dir.path().join("Skillfile.lock")).unwrap();
    assert!(lock_text.contains("code-refactorer"));
    assert!(lock_text.contains("requesting-code-review"));

    // Cache populated
    assert!(dir
        .path()
        .join(".skillfile/cache/agents/code-refactorer")
        .is_dir());

    // NOT deployed (sync only)
    assert!(!dir.path().join(".claude").exists());
}

#[test]
fn install_golden_path() {
    require_github_token();
    let dir = make_repo();

    sf(dir.path()).arg("install").assert().success();

    // Lock file written
    assert!(dir.path().join("Skillfile.lock").exists());
    let lock_text = std::fs::read_to_string(dir.path().join("Skillfile.lock")).unwrap();
    assert!(lock_text.contains("code-refactorer"));
    assert!(lock_text.contains("requesting-code-review"));

    // Cache populated
    assert!(dir
        .path()
        .join(".skillfile/cache/agents/code-refactorer")
        .is_dir());
    assert!(dir
        .path()
        .join(".skillfile/cache/skills/requesting-code-review")
        .is_dir());

    // Deployed to local .claude/
    let agent_file = dir.path().join(".claude/agents/code-refactorer.md");
    assert!(agent_file.exists());

    // Content is real markdown
    let content = std::fs::read_to_string(&agent_file).unwrap();
    assert!(content.len() > 10, "deployed file should have content");
}

#[test]
fn install_dry_run() {
    require_github_token();
    let dir = make_repo();

    sf(dir.path())
        .args(["install", "--dry-run"])
        .assert()
        .success()
        .stderr(predicate::str::contains("dry-run"));

    // Nothing written
    assert!(
        !dir.path().join("Skillfile.lock").exists(),
        "lock should not be written in dry-run"
    );
    assert!(
        !dir.path().join(".claude").exists(),
        ".claude should not be created in dry-run"
    );
}

#[test]
fn install_update() {
    require_github_token();
    let dir = make_repo();

    // First install
    sf(dir.path()).arg("install").assert().success();

    // Update (SHAs should stay the same since repo hasn't changed)
    sf(dir.path())
        .args(["install", "--update"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Done"));
}

#[test]
fn pin_then_unpin() {
    require_github_token();
    let dir = make_repo();

    // Install first
    sf(dir.path()).arg("install").assert().success();

    // Modify installed file
    let agent_file = dir.path().join(".claude/agents/code-refactorer.md");
    let original = std::fs::read_to_string(&agent_file).unwrap();
    std::fs::write(&agent_file, format!("{original}\n## My custom section\n")).unwrap();

    // Pin
    sf(dir.path())
        .args(["pin", "code-refactorer"])
        .assert()
        .success();

    let patch_file = dir
        .path()
        .join(".skillfile/patches/agents/code-refactorer.patch");
    assert!(patch_file.exists(), "patch file should exist after pin");

    // Unpin
    sf(dir.path())
        .args(["unpin", "code-refactorer"])
        .assert()
        .success();

    assert!(
        !patch_file.exists(),
        "patch file should be removed after unpin"
    );

    // Installed file should be back to original (unpin reinstalls)
    let restored = std::fs::read_to_string(&agent_file).unwrap();
    assert_eq!(restored, original, "file should be restored to upstream");
}

#[test]
fn status_after_install() {
    require_github_token();
    let dir = make_repo();

    sf(dir.path()).arg("install").assert().success();

    sf(dir.path())
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("code-refactorer"))
        .stdout(predicate::str::contains("requesting-code-review"));
}
