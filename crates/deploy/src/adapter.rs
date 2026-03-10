use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use skillfile_core::models::{Entry, InstallOptions, Scope};
use skillfile_core::patch::walkdir;
use skillfile_sources::strategy::is_dir_entry;

// ---------------------------------------------------------------------------
// PlatformAdapter trait — the core abstraction for tool-specific deployment
// ---------------------------------------------------------------------------

/// How a directory entry is deployed to a platform's target directory.
///
/// - `Flat`: each `.md` placed individually in `target_dir/` (e.g. claude-code agents)
/// - `Nested`: directory placed as `target_dir/<name>/` (e.g. all skill adapters)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirInstallMode {
    Flat,
    Nested,
}

/// The deployment result: a map of `{patch_key: installed_path}`.
///
/// Keys match the relative paths used in `.skillfile/patches/` so patch lookups
/// work correctly:
/// - Single-file entries: key is `"{name}.md"`
/// - Directory entries: keys are paths relative to the source directory
pub type DeployResult = HashMap<String, PathBuf>;

/// Contract for deploying skill/agent files to a specific AI tool's directory.
///
/// Each AI tool (Claude Code, Gemini CLI, Codex, etc.) has its own convention
/// for where skills and agents live on disk. A `PlatformAdapter` encapsulates
/// that knowledge.
///
/// The trait is object-safe so adapters can be stored in a heterogeneous registry.
pub trait PlatformAdapter: Send + Sync + fmt::Debug {
    /// The adapter identifier (e.g. `"claude-code"`, `"gemini-cli"`).
    fn name(&self) -> &str;

    /// Whether this platform supports the given entity type (e.g. `"skill"`, `"agent"`).
    fn supports(&self, entity_type: &str) -> bool;

    /// Resolve the absolute target directory for an entity type + scope.
    fn target_dir(&self, entity_type: &str, scope: Scope, repo_root: &Path) -> PathBuf;

    /// The install mode for directory entries of this entity type.
    fn dir_mode(&self, entity_type: &str) -> Option<DirInstallMode>;

    /// Deploy a single entry from `source` to its platform-specific location.
    ///
    /// Returns `{patch_key: installed_path}` for every file that was placed.
    /// Returns an empty map for dry-run or when deployment is skipped.
    fn deploy_entry(
        &self,
        entry: &Entry,
        source: &Path,
        scope: Scope,
        repo_root: &Path,
        opts: &InstallOptions,
    ) -> DeployResult;

    /// The installed path for a single-file entry.
    fn installed_path(&self, entry: &Entry, scope: Scope, repo_root: &Path) -> PathBuf;

    /// Map of `{relative_path: absolute_path}` for all installed files of a directory entry.
    fn installed_dir_files(
        &self,
        entry: &Entry,
        scope: Scope,
        repo_root: &Path,
    ) -> HashMap<String, PathBuf>;
}

// ---------------------------------------------------------------------------
// EntityConfig — per-entity-type path configuration
// ---------------------------------------------------------------------------

/// Paths and install mode for one entity type within a platform.
#[derive(Debug, Clone)]
pub struct EntityConfig {
    pub global_path: String,
    pub local_path: String,
    pub dir_mode: DirInstallMode,
}

// ---------------------------------------------------------------------------
// FileSystemAdapter — the concrete implementation of PlatformAdapter
// ---------------------------------------------------------------------------

/// Filesystem-based platform adapter.
///
/// Each instance is configured with a name and a map of `EntityConfig`s.
/// All three built-in adapters (claude-code, gemini-cli, codex) are instances
/// of this struct with different configurations — the `PlatformAdapter` trait
/// allows alternative implementations if needed.
#[derive(Debug, Clone)]
pub struct FileSystemAdapter {
    name: String,
    entities: HashMap<String, EntityConfig>,
}

impl FileSystemAdapter {
    pub fn new(name: &str, entities: HashMap<String, EntityConfig>) -> Self {
        Self {
            name: name.to_string(),
            entities,
        }
    }
}

impl PlatformAdapter for FileSystemAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn supports(&self, entity_type: &str) -> bool {
        self.entities.contains_key(entity_type)
    }

    fn target_dir(&self, entity_type: &str, scope: Scope, repo_root: &Path) -> PathBuf {
        let config = &self.entities[entity_type];
        let raw = match scope {
            Scope::Global => &config.global_path,
            Scope::Local => &config.local_path,
        };
        if raw.starts_with('~') {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
            home.join(raw.strip_prefix("~/").unwrap_or(raw))
        } else {
            repo_root.join(raw)
        }
    }

    fn dir_mode(&self, entity_type: &str) -> Option<DirInstallMode> {
        self.entities.get(entity_type).map(|c| c.dir_mode)
    }

    fn deploy_entry(
        &self,
        entry: &Entry,
        source: &Path,
        scope: Scope,
        repo_root: &Path,
        opts: &InstallOptions,
    ) -> DeployResult {
        let target_dir = self.target_dir(&entry.entity_type, scope, repo_root);
        let is_dir = is_dir_entry(entry);

        if is_dir
            && self
                .entities
                .get(&entry.entity_type)
                .is_some_and(|c| c.dir_mode == DirInstallMode::Flat)
        {
            return deploy_flat(source, &target_dir, opts);
        }

        let dest = if is_dir {
            target_dir.join(&entry.name)
        } else {
            target_dir.join(format!("{}.md", entry.name))
        };

        if !place_file(source, &dest, is_dir, opts) || opts.dry_run {
            return HashMap::new();
        }

        if is_dir {
            let mut result = HashMap::new();
            for file in walkdir(source) {
                if file.file_name().map_or(true, |n| n == ".meta") {
                    continue;
                }
                if let Ok(rel) = file.strip_prefix(source) {
                    result.insert(rel.to_string_lossy().to_string(), dest.join(rel));
                }
            }
            result
        } else {
            HashMap::from([(format!("{}.md", entry.name), dest)])
        }
    }

    fn installed_path(&self, entry: &Entry, scope: Scope, repo_root: &Path) -> PathBuf {
        self.target_dir(&entry.entity_type, scope, repo_root)
            .join(format!("{}.md", entry.name))
    }

    fn installed_dir_files(
        &self,
        entry: &Entry,
        scope: Scope,
        repo_root: &Path,
    ) -> HashMap<String, PathBuf> {
        let target_dir = self.target_dir(&entry.entity_type, scope, repo_root);
        let mode = self
            .entities
            .get(&entry.entity_type)
            .map(|c| c.dir_mode)
            .unwrap_or(DirInstallMode::Nested);

        if mode == DirInstallMode::Nested {
            let installed_dir = target_dir.join(&entry.name);
            if !installed_dir.is_dir() {
                return HashMap::new();
            }
            let mut result = HashMap::new();
            for file in walkdir(&installed_dir) {
                if let Ok(rel) = file.strip_prefix(&installed_dir) {
                    result.insert(rel.to_string_lossy().to_string(), file);
                }
            }
            result
        } else {
            // Flat: keys are relative-from-vdir so they match patch lookup keys
            let vdir = skillfile_sources::sync::vendor_dir_for(entry, repo_root);
            if !vdir.is_dir() {
                return HashMap::new();
            }
            let mut result = HashMap::new();
            for file in walkdir(&vdir) {
                if file
                    .extension()
                    .map_or(true, |ext| ext.to_string_lossy() != "md")
                {
                    continue;
                }
                if let Ok(rel) = file.strip_prefix(&vdir) {
                    let dest = target_dir.join(file.file_name().unwrap_or_default());
                    if dest.exists() {
                        result.insert(rel.to_string_lossy().to_string(), dest);
                    }
                }
            }
            result
        }
    }
}

// ---------------------------------------------------------------------------
// Deployment helpers (used by FileSystemAdapter)
// ---------------------------------------------------------------------------

/// Deploy each `.md` in `source_dir` as an individual file in `target_dir` (flat mode).
fn deploy_flat(source_dir: &Path, target_dir: &Path, opts: &InstallOptions) -> DeployResult {
    let mut md_files: Vec<PathBuf> = walkdir(source_dir)
        .into_iter()
        .filter(|f| f.extension().is_some_and(|ext| ext == "md"))
        .collect();
    md_files.sort();

    if opts.dry_run {
        for src in &md_files {
            if let Some(name) = src.file_name() {
                eprintln!(
                    "  {} -> {} [copy, dry-run]",
                    name.to_string_lossy(),
                    target_dir.join(name).display()
                );
            }
        }
        return HashMap::new();
    }

    std::fs::create_dir_all(target_dir).ok();
    let mut result = HashMap::new();
    for src in &md_files {
        let Some(name) = src.file_name() else {
            continue;
        };
        let dest = target_dir.join(name);
        if !opts.overwrite && dest.is_file() {
            continue;
        }
        if dest.exists() {
            std::fs::remove_file(&dest).ok();
        }
        if std::fs::copy(src, &dest).is_ok() {
            eprintln!("  {} -> {}", name.to_string_lossy(), dest.display());
            if let Ok(rel) = src.strip_prefix(source_dir) {
                result.insert(rel.to_string_lossy().to_string(), dest);
            }
        }
    }
    result
}

/// Copy `source` to `dest`. Returns `true` if placed, `false` if skipped.
fn place_file(source: &Path, dest: &Path, is_dir: bool, opts: &InstallOptions) -> bool {
    if !opts.overwrite && !opts.dry_run {
        if is_dir && dest.is_dir() {
            return false;
        }
        if !is_dir && dest.is_file() {
            return false;
        }
    }

    let label = format!(
        "  {} -> {}",
        source.file_name().unwrap_or_default().to_string_lossy(),
        dest.display()
    );

    if opts.dry_run {
        eprintln!("{label} [copy, dry-run]");
        return true;
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    // Remove existing
    if dest.exists() || dest.is_symlink() {
        if dest.is_dir() {
            std::fs::remove_dir_all(dest).ok();
        } else {
            std::fs::remove_file(dest).ok();
        }
    }

    if is_dir {
        copy_dir_recursive(source, dest).ok();
    } else {
        std::fs::copy(source, dest).ok();
    }

    eprintln!("{label}");
    true
}

/// Recursively copy a directory tree.
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// AdapterRegistry — injectable, testable collection of platform adapters
// ---------------------------------------------------------------------------

/// A collection of platform adapters, indexed by name.
///
/// The registry owns the adapters and provides lookup by name. It can be
/// constructed with the built-in adapters via [`AdapterRegistry::builtin()`],
/// or built manually for testing.
pub struct AdapterRegistry {
    adapters: HashMap<String, Box<dyn PlatformAdapter>>,
}

impl AdapterRegistry {
    /// Create a registry from a vec of boxed adapters.
    pub fn new(adapters: Vec<Box<dyn PlatformAdapter>>) -> Self {
        let map = adapters
            .into_iter()
            .map(|a| (a.name().to_string(), a))
            .collect();
        Self { adapters: map }
    }

    /// Create the built-in registry with all known platform adapters.
    pub fn builtin() -> Self {
        Self::new(vec![
            Box::new(claude_code_adapter()),
            Box::new(gemini_cli_adapter()),
            Box::new(codex_adapter()),
        ])
    }

    /// Look up an adapter by name.
    pub fn get(&self, name: &str) -> Option<&dyn PlatformAdapter> {
        self.adapters.get(name).map(|b| &**b)
    }

    /// Check if an adapter with this name exists.
    pub fn contains(&self, name: &str) -> bool {
        self.adapters.contains_key(name)
    }

    /// Sorted list of all adapter names.
    pub fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.adapters.keys().map(|s| s.as_str()).collect();
        names.sort();
        names
    }
}

impl fmt::Debug for AdapterRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdapterRegistry")
            .field("adapters", &self.names())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Built-in adapters
// ---------------------------------------------------------------------------

fn claude_code_adapter() -> FileSystemAdapter {
    FileSystemAdapter::new(
        "claude-code",
        HashMap::from([
            (
                "agent".to_string(),
                EntityConfig {
                    global_path: "~/.claude/agents".into(),
                    local_path: ".claude/agents".into(),
                    dir_mode: DirInstallMode::Flat,
                },
            ),
            (
                "skill".to_string(),
                EntityConfig {
                    global_path: "~/.claude/skills".into(),
                    local_path: ".claude/skills".into(),
                    dir_mode: DirInstallMode::Nested,
                },
            ),
        ]),
    )
}

fn gemini_cli_adapter() -> FileSystemAdapter {
    FileSystemAdapter::new(
        "gemini-cli",
        HashMap::from([
            (
                "agent".to_string(),
                EntityConfig {
                    global_path: "~/.gemini/agents".into(),
                    local_path: ".gemini/agents".into(),
                    dir_mode: DirInstallMode::Flat,
                },
            ),
            (
                "skill".to_string(),
                EntityConfig {
                    global_path: "~/.gemini/skills".into(),
                    local_path: ".gemini/skills".into(),
                    dir_mode: DirInstallMode::Nested,
                },
            ),
        ]),
    )
}

fn codex_adapter() -> FileSystemAdapter {
    FileSystemAdapter::new(
        "codex",
        HashMap::from([(
            "skill".to_string(),
            EntityConfig {
                global_path: "~/.codex/skills".into(),
                local_path: ".codex/skills".into(),
                dir_mode: DirInstallMode::Nested,
            },
        )]),
    )
}

// ---------------------------------------------------------------------------
// Global registry accessor (backward-compatible convenience)
// ---------------------------------------------------------------------------

/// Get the global adapter registry (lazily initialized).
pub fn adapters() -> &'static AdapterRegistry {
    static REGISTRY: OnceLock<AdapterRegistry> = OnceLock::new();
    REGISTRY.get_or_init(AdapterRegistry::builtin)
}

/// Sorted list of known adapter names.
pub fn known_adapters() -> Vec<&'static str> {
    adapters().names()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Trait compliance: every registered adapter satisfies PlatformAdapter --

    #[test]
    fn all_builtin_adapters_in_registry() {
        let reg = adapters();
        assert!(reg.contains("claude-code"));
        assert!(reg.contains("gemini-cli"));
        assert!(reg.contains("codex"));
    }

    #[test]
    fn known_adapters_contains_all() {
        let names = known_adapters();
        assert!(names.contains(&"claude-code"));
        assert!(names.contains(&"gemini-cli"));
        assert!(names.contains(&"codex"));
        assert_eq!(names.len(), 3);
    }

    #[test]
    fn adapter_name_matches_registry_key() {
        let reg = adapters();
        for name in reg.names() {
            let adapter = reg.get(name).unwrap();
            assert_eq!(adapter.name(), name);
        }
    }

    #[test]
    fn registry_get_unknown_returns_none() {
        assert!(adapters().get("unknown-tool").is_none());
    }

    // -- supports() --

    #[test]
    fn claude_code_supports_agent_and_skill() {
        let a = adapters().get("claude-code").unwrap();
        assert!(a.supports("agent"));
        assert!(a.supports("skill"));
        assert!(!a.supports("hook"));
    }

    #[test]
    fn gemini_cli_supports_agent_and_skill() {
        let a = adapters().get("gemini-cli").unwrap();
        assert!(a.supports("agent"));
        assert!(a.supports("skill"));
    }

    #[test]
    fn codex_supports_skill_not_agent() {
        let a = adapters().get("codex").unwrap();
        assert!(a.supports("skill"));
        assert!(!a.supports("agent"));
    }

    // -- target_dir() --

    #[test]
    fn local_target_dir_claude_code() {
        let tmp = PathBuf::from("/tmp/test");
        let a = adapters().get("claude-code").unwrap();
        assert_eq!(
            a.target_dir("agent", Scope::Local, &tmp),
            tmp.join(".claude/agents")
        );
        assert_eq!(
            a.target_dir("skill", Scope::Local, &tmp),
            tmp.join(".claude/skills")
        );
    }

    #[test]
    fn local_target_dir_gemini_cli() {
        let tmp = PathBuf::from("/tmp/test");
        let a = adapters().get("gemini-cli").unwrap();
        assert_eq!(
            a.target_dir("agent", Scope::Local, &tmp),
            tmp.join(".gemini/agents")
        );
        assert_eq!(
            a.target_dir("skill", Scope::Local, &tmp),
            tmp.join(".gemini/skills")
        );
    }

    #[test]
    fn local_target_dir_codex() {
        let tmp = PathBuf::from("/tmp/test");
        let a = adapters().get("codex").unwrap();
        assert_eq!(
            a.target_dir("skill", Scope::Local, &tmp),
            tmp.join(".codex/skills")
        );
    }

    #[test]
    fn global_target_dir_is_absolute() {
        let a = adapters().get("claude-code").unwrap();
        let result = a.target_dir("agent", Scope::Global, Path::new("/tmp"));
        assert!(result.is_absolute());
        assert!(result.to_string_lossy().ends_with(".claude/agents"));
    }

    #[test]
    fn global_target_dir_gemini_cli_skill() {
        let a = adapters().get("gemini-cli").unwrap();
        let result = a.target_dir("skill", Scope::Global, Path::new("/tmp"));
        assert!(result.is_absolute());
        assert!(result.to_string_lossy().ends_with(".gemini/skills"));
    }

    #[test]
    fn global_target_dir_codex_skill() {
        let a = adapters().get("codex").unwrap();
        let result = a.target_dir("skill", Scope::Global, Path::new("/tmp"));
        assert!(result.is_absolute());
        assert!(result.to_string_lossy().ends_with(".codex/skills"));
    }

    // -- dir_mode --

    #[test]
    fn claude_code_dir_modes() {
        let a = adapters().get("claude-code").unwrap();
        assert_eq!(a.dir_mode("agent"), Some(DirInstallMode::Flat));
        assert_eq!(a.dir_mode("skill"), Some(DirInstallMode::Nested));
    }

    #[test]
    fn gemini_cli_dir_modes() {
        let a = adapters().get("gemini-cli").unwrap();
        assert_eq!(a.dir_mode("agent"), Some(DirInstallMode::Flat));
        assert_eq!(a.dir_mode("skill"), Some(DirInstallMode::Nested));
    }

    #[test]
    fn codex_dir_mode() {
        let a = adapters().get("codex").unwrap();
        assert_eq!(a.dir_mode("skill"), Some(DirInstallMode::Nested));
    }

    // -- Custom adapter extensibility --

    #[test]
    fn custom_adapter_via_registry() {
        let custom = FileSystemAdapter::new(
            "my-tool",
            HashMap::from([(
                "skill".to_string(),
                EntityConfig {
                    global_path: "~/.my-tool/skills".into(),
                    local_path: ".my-tool/skills".into(),
                    dir_mode: DirInstallMode::Nested,
                },
            )]),
        );
        let registry = AdapterRegistry::new(vec![Box::new(custom)]);
        let a = registry.get("my-tool").unwrap();
        assert!(a.supports("skill"));
        assert!(!a.supports("agent"));
        assert_eq!(registry.names(), vec!["my-tool"]);
    }

    // -- deploy_entry key contract --

    #[test]
    fn deploy_entry_single_file_key_matches_patch_convention() {
        use skillfile_core::models::SourceFields;

        let dir = tempfile::tempdir().unwrap();
        let source_dir = dir.path().join(".skillfile/cache/agents/test");
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::write(source_dir.join("agent.md"), "# Agent\n").unwrap();
        let source = source_dir.join("agent.md");

        let entry = Entry {
            entity_type: "agent".into(),
            name: "test".into(),
            source: SourceFields::Github {
                owner_repo: "o/r".into(),
                path_in_repo: "agents/agent.md".into(),
                ref_: "main".into(),
            },
        };
        let a = adapters().get("claude-code").unwrap();
        let result = a.deploy_entry(
            &entry,
            &source,
            Scope::Local,
            dir.path(),
            &InstallOptions::default(),
        );
        assert!(
            result.contains_key("test.md"),
            "Single-file key must be 'test.md', got {:?}",
            result.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn deploy_entry_dir_keys_match_source_relative_paths() {
        use skillfile_core::models::SourceFields;

        let dir = tempfile::tempdir().unwrap();
        let source_dir = dir.path().join(".skillfile/cache/skills/my-skill");
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::write(source_dir.join("SKILL.md"), "# Skill\n").unwrap();
        std::fs::write(source_dir.join("examples.md"), "# Examples\n").unwrap();

        let entry = Entry {
            entity_type: "skill".into(),
            name: "my-skill".into(),
            source: SourceFields::Github {
                owner_repo: "o/r".into(),
                path_in_repo: "skills/my-skill".into(),
                ref_: "main".into(),
            },
        };
        let a = adapters().get("claude-code").unwrap();
        let result = a.deploy_entry(
            &entry,
            &source_dir,
            Scope::Local,
            dir.path(),
            &InstallOptions::default(),
        );
        assert!(result.contains_key("SKILL.md"));
        assert!(result.contains_key("examples.md"));
    }
}
