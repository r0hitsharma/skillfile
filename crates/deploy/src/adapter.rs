use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use skillfile_core::models::{Entry, InstallOptions};
use skillfile_sources::strategy::is_dir_entry;

/// How a directory entry is deployed.
///
/// - `Flat`: each .md placed individually in target_dir/ (e.g. claude-code agents)
/// - `Nested`: directory placed as target_dir/<name>/ (e.g. all skill adapters)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirInstallMode {
    Flat,
    Nested,
}

/// Paths and install mode for one entity type within a platform.
#[derive(Debug, Clone)]
pub struct EntityConfig {
    pub global_path: String,
    pub local_path: String,
    pub dir_mode: DirInstallMode,
}

/// Filesystem-based platform adapter. Each instance is configured with a name
/// and a map of entity configs. All three built-in adapters are instances of this
/// struct — no subclassing needed.
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

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn supports(&self, entity_type: &str) -> bool {
        self.entities.contains_key(entity_type)
    }

    /// Resolve the absolute deploy directory for (entity_type, scope, repo_root).
    pub fn target_dir(&self, entity_type: &str, scope: &str, repo_root: &Path) -> PathBuf {
        let config = &self.entities[entity_type];
        let raw = if scope == "global" {
            &config.global_path
        } else {
            &config.local_path
        };
        if raw.starts_with('~') {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
            home.join(raw.strip_prefix("~/").unwrap_or(raw))
        } else {
            repo_root.join(raw)
        }
    }

    /// Get the dir_mode for an entity type.
    pub fn dir_mode(&self, entity_type: &str) -> Option<DirInstallMode> {
        self.entities.get(entity_type).map(|c| c.dir_mode)
    }

    /// Deploy a single entry. Returns {relative_key: installed_path}.
    ///
    /// Keys match the relative paths used in .skillfile/patches/ so patch lookups
    /// work. For single-file entries, key is "{name}.md". For directory entries,
    /// keys are paths relative to the source directory.
    pub fn deploy_entry(
        &self,
        entry: &Entry,
        source: &Path,
        scope: &str,
        repo_root: &Path,
        opts: &InstallOptions,
    ) -> HashMap<String, PathBuf> {
        let target_dir = self.target_dir(&entry.entity_type, scope, repo_root);
        let is_dir = is_dir_entry(entry);

        if is_dir
            && self
                .entities
                .get(&entry.entity_type)
                .is_some_and(|c| c.dir_mode == DirInstallMode::Flat)
        {
            return self.deploy_flat(source, &target_dir, opts);
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
            if let Ok(walker) = walkdir(source) {
                for file in walker {
                    if file.file_name().map_or(true, |n| n == ".meta") {
                        continue;
                    }
                    if let Ok(rel) = file.strip_prefix(source) {
                        result.insert(rel.to_string_lossy().to_string(), dest.join(rel));
                    }
                }
            }
            result
        } else {
            let mut result = HashMap::new();
            result.insert(format!("{}.md", entry.name), dest);
            result
        }
    }

    /// Get the installed path for a single-file entry.
    pub fn installed_path(&self, entry: &Entry, scope: &str, repo_root: &Path) -> PathBuf {
        self.target_dir(&entry.entity_type, scope, repo_root)
            .join(format!("{}.md", entry.name))
    }

    /// Get installed files for a directory entry. Returns {relative_path: absolute_path}.
    pub fn installed_dir_files(
        &self,
        entry: &Entry,
        scope: &str,
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
            if let Ok(walker) = walkdir(&installed_dir) {
                for file in walker {
                    if let Ok(rel) = file.strip_prefix(&installed_dir) {
                        result.insert(rel.to_string_lossy().to_string(), file);
                    }
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
            if let Ok(walker) = walkdir(&vdir) {
                for file in walker {
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
            }
            result
        }
    }

    /// Deploy each .md in source_dir as an individual file in target_dir (flat mode).
    fn deploy_flat(
        &self,
        source_dir: &Path,
        target_dir: &Path,
        opts: &InstallOptions,
    ) -> HashMap<String, PathBuf> {
        let mut md_files: Vec<PathBuf> = Vec::new();
        if let Ok(walker) = walkdir(source_dir) {
            for file in walker {
                if file
                    .extension()
                    .is_some_and(|ext| ext.to_string_lossy() == "md")
                {
                    md_files.push(file);
                }
            }
        }
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
}

/// Copy source to dest. Returns true if placed, false if skipped.
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

/// Walk a directory recursively and return all file paths (non-directory entries).
fn walkdir(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    walkdir_inner(dir, &mut files)?;
    Ok(files)
}

fn walkdir_inner(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walkdir_inner(&path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Registry — one instance per tool
// ---------------------------------------------------------------------------

fn claude_code_adapter() -> FileSystemAdapter {
    let mut entities = HashMap::new();
    entities.insert(
        "agent".into(),
        EntityConfig {
            global_path: "~/.claude/agents".into(),
            local_path: ".claude/agents".into(),
            dir_mode: DirInstallMode::Flat,
        },
    );
    entities.insert(
        "skill".into(),
        EntityConfig {
            global_path: "~/.claude/skills".into(),
            local_path: ".claude/skills".into(),
            dir_mode: DirInstallMode::Nested,
        },
    );
    FileSystemAdapter::new("claude-code", entities)
}

fn gemini_cli_adapter() -> FileSystemAdapter {
    let mut entities = HashMap::new();
    entities.insert(
        "agent".into(),
        EntityConfig {
            global_path: "~/.gemini/agents".into(),
            local_path: ".gemini/agents".into(),
            dir_mode: DirInstallMode::Flat,
        },
    );
    entities.insert(
        "skill".into(),
        EntityConfig {
            global_path: "~/.gemini/skills".into(),
            local_path: ".gemini/skills".into(),
            dir_mode: DirInstallMode::Nested,
        },
    );
    FileSystemAdapter::new("gemini-cli", entities)
}

fn codex_adapter() -> FileSystemAdapter {
    let mut entities = HashMap::new();
    entities.insert(
        "skill".into(),
        EntityConfig {
            global_path: "~/.codex/skills".into(),
            local_path: ".codex/skills".into(),
            dir_mode: DirInstallMode::Nested,
        },
    );
    FileSystemAdapter::new("codex", entities)
}

/// Get the global adapter registry (lazily initialized).
pub fn adapters() -> &'static HashMap<String, FileSystemAdapter> {
    static ADAPTERS: OnceLock<HashMap<String, FileSystemAdapter>> = OnceLock::new();
    ADAPTERS.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("claude-code".into(), claude_code_adapter());
        m.insert("gemini-cli".into(), gemini_cli_adapter());
        m.insert("codex".into(), codex_adapter());
        m
    })
}

/// List of known adapter names.
pub fn known_adapters() -> Vec<&'static str> {
    let a = adapters();
    let mut names: Vec<&str> = a.keys().map(|s| s.as_str()).collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Protocol compliance: every registered adapter is a FileSystemAdapter --

    #[test]
    fn claude_code_adapter_in_registry() {
        assert!(adapters().contains_key("claude-code"));
    }

    #[test]
    fn gemini_cli_adapter_in_registry() {
        assert!(adapters().contains_key("gemini-cli"));
    }

    #[test]
    fn codex_adapter_in_registry() {
        assert!(adapters().contains_key("codex"));
    }

    // -- Registry completeness --

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
        for (key, adapter) in adapters() {
            assert_eq!(adapter.name(), key);
        }
    }

    // -- supports() --

    #[test]
    fn claude_code_supports_agent() {
        assert!(adapters()["claude-code"].supports("agent"));
    }

    #[test]
    fn claude_code_supports_skill() {
        assert!(adapters()["claude-code"].supports("skill"));
    }

    #[test]
    fn gemini_cli_supports_agent() {
        assert!(adapters()["gemini-cli"].supports("agent"));
    }

    #[test]
    fn gemini_cli_supports_skill() {
        assert!(adapters()["gemini-cli"].supports("skill"));
    }

    #[test]
    fn codex_supports_skill() {
        assert!(adapters()["codex"].supports("skill"));
    }

    #[test]
    fn codex_does_not_support_agents() {
        assert!(!adapters()["codex"].supports("agent"));
    }

    #[test]
    fn adapters_do_not_support_unknown_entity() {
        assert!(!adapters()["claude-code"].supports("hook"));
        assert!(!adapters()["gemini-cli"].supports("hook"));
        assert!(!adapters()["codex"].supports("hook"));
    }

    // -- target_dir() local scope --

    #[test]
    fn local_target_dir_claude_code_agent() {
        let tmp = PathBuf::from("/tmp/test");
        let a = &adapters()["claude-code"];
        assert_eq!(
            a.target_dir("agent", "local", &tmp),
            tmp.join(".claude/agents")
        );
    }

    #[test]
    fn local_target_dir_claude_code_skill() {
        let tmp = PathBuf::from("/tmp/test");
        let a = &adapters()["claude-code"];
        assert_eq!(
            a.target_dir("skill", "local", &tmp),
            tmp.join(".claude/skills")
        );
    }

    #[test]
    fn local_target_dir_gemini_cli_agent() {
        let tmp = PathBuf::from("/tmp/test");
        let a = &adapters()["gemini-cli"];
        assert_eq!(
            a.target_dir("agent", "local", &tmp),
            tmp.join(".gemini/agents")
        );
    }

    #[test]
    fn local_target_dir_gemini_cli_skill() {
        let tmp = PathBuf::from("/tmp/test");
        let a = &adapters()["gemini-cli"];
        assert_eq!(
            a.target_dir("skill", "local", &tmp),
            tmp.join(".gemini/skills")
        );
    }

    #[test]
    fn local_target_dir_codex_skill() {
        let tmp = PathBuf::from("/tmp/test");
        let a = &adapters()["codex"];
        assert_eq!(
            a.target_dir("skill", "local", &tmp),
            tmp.join(".codex/skills")
        );
    }

    // -- target_dir() global scope --

    #[test]
    fn global_target_dir_is_absolute() {
        let a = &adapters()["claude-code"];
        let result = a.target_dir("agent", "global", Path::new("/tmp"));
        assert!(result.is_absolute());
        assert!(result.to_string_lossy().ends_with(".claude/agents"));
    }

    #[test]
    fn global_target_dir_gemini_cli_skill() {
        let a = &adapters()["gemini-cli"];
        let result = a.target_dir("skill", "global", Path::new("/tmp"));
        assert!(result.is_absolute());
        assert!(result.to_string_lossy().ends_with(".gemini/skills"));
    }

    #[test]
    fn global_target_dir_codex_skill() {
        let a = &adapters()["codex"];
        let result = a.target_dir("skill", "global", Path::new("/tmp"));
        assert!(result.is_absolute());
        assert!(result.to_string_lossy().ends_with(".codex/skills"));
    }

    // -- dir_mode --

    #[test]
    fn claude_code_agent_flat() {
        assert_eq!(
            adapters()["claude-code"].dir_mode("agent"),
            Some(DirInstallMode::Flat)
        );
    }

    #[test]
    fn claude_code_skill_nested() {
        assert_eq!(
            adapters()["claude-code"].dir_mode("skill"),
            Some(DirInstallMode::Nested)
        );
    }

    #[test]
    fn gemini_cli_agent_flat() {
        assert_eq!(
            adapters()["gemini-cli"].dir_mode("agent"),
            Some(DirInstallMode::Flat)
        );
    }

    #[test]
    fn gemini_cli_skill_nested() {
        assert_eq!(
            adapters()["gemini-cli"].dir_mode("skill"),
            Some(DirInstallMode::Nested)
        );
    }

    #[test]
    fn codex_skill_nested() {
        assert_eq!(
            adapters()["codex"].dir_mode("skill"),
            Some(DirInstallMode::Nested)
        );
    }

    // -- Custom adapter extensibility --

    #[test]
    fn custom_filesystem_adapter() {
        let mut entities = HashMap::new();
        entities.insert(
            "skill".into(),
            EntityConfig {
                global_path: "~/.my-tool/skills".into(),
                local_path: ".my-tool/skills".into(),
                dir_mode: DirInstallMode::Nested,
            },
        );
        let adapter = FileSystemAdapter::new("my-tool", entities);
        assert!(adapter.supports("skill"));
        assert!(!adapter.supports("agent"));
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
        let result = adapters()["claude-code"].deploy_entry(
            &entry,
            &source,
            "local",
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
        let result = adapters()["claude-code"].deploy_entry(
            &entry,
            &source_dir,
            "local",
            dir.path(),
            &InstallOptions::default(),
        );
        assert!(result.contains_key("SKILL.md"));
        assert!(result.contains_key("examples.md"));
    }
}
