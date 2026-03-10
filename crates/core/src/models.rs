use std::fmt;

use serde::{Deserialize, Serialize};

pub const DEFAULT_REF: &str = "main";

// ---------------------------------------------------------------------------
// Scope — typed replacement for bare "global"/"local" strings
// ---------------------------------------------------------------------------

/// Install scope: either the user's global config directory or the local repo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    Global,
    Local,
}

impl Scope {
    /// All valid scope values, in alphabetical order.
    pub const ALL: &[Scope] = &[Scope::Global, Scope::Local];

    /// Parse a scope string. Returns `None` for unrecognised values.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "global" => Some(Scope::Global),
            "local" => Some(Scope::Local),
            _ => None,
        }
    }

    /// The canonical string representation (used in Skillfile format).
    pub fn as_str(&self) -> &'static str {
        match self {
            Scope::Global => "global",
            Scope::Local => "local",
        }
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// SourceFields — making illegal states unrepresentable
// ---------------------------------------------------------------------------

/// Source-specific fields, making illegal states unrepresentable.
///
/// Instead of optional fields on Entry (Python's approach), we use an enum
/// so each variant carries exactly the fields it needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceFields {
    Github {
        owner_repo: String,
        path_in_repo: String,
        ref_: String,
    },
    Local {
        path: String,
    },
    Url {
        url: String,
    },
}

impl SourceFields {
    /// The source type identifier used in the Skillfile format and lock keys.
    pub fn source_type(&self) -> &str {
        match self {
            SourceFields::Github { .. } => "github",
            SourceFields::Local { .. } => "local",
            SourceFields::Url { .. } => "url",
        }
    }

    /// Access GitHub-specific fields. Returns `None` for non-GitHub sources.
    pub fn as_github(&self) -> Option<(&str, &str, &str)> {
        match self {
            SourceFields::Github {
                owner_repo,
                path_in_repo,
                ref_,
            } => Some((owner_repo, path_in_repo, ref_)),
            _ => None,
        }
    }

    /// Access the local path. Returns `None` for non-Local sources.
    pub fn as_local(&self) -> Option<&str> {
        match self {
            SourceFields::Local { path } => Some(path),
            _ => None,
        }
    }

    /// Access the URL. Returns `None` for non-Url sources.
    pub fn as_url(&self) -> Option<&str> {
        match self {
            SourceFields::Url { url } => Some(url),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Entry — a single manifest entry
// ---------------------------------------------------------------------------

/// A single entry in the Skillfile manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub entity_type: String,
    pub name: String,
    pub source: SourceFields,
}

impl Entry {
    /// Shorthand: the source type identifier.
    pub fn source_type(&self) -> &str {
        self.source.source_type()
    }

    // Legacy convenience accessors — delegate to SourceFields.
    // These return "" for inapplicable variants to keep backward compatibility
    // with existing callers. Prefer matching on `entry.source` directly.

    pub fn owner_repo(&self) -> &str {
        self.source.as_github().map(|(or, _, _)| or).unwrap_or("")
    }

    pub fn path_in_repo(&self) -> &str {
        self.source.as_github().map(|(_, pir, _)| pir).unwrap_or("")
    }

    pub fn ref_(&self) -> &str {
        self.source.as_github().map(|(_, _, r)| r).unwrap_or("")
    }

    pub fn local_path(&self) -> &str {
        self.source.as_local().unwrap_or("")
    }

    pub fn url(&self) -> &str {
        self.source.as_url().unwrap_or("")
    }
}

impl fmt::Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}/{}",
            self.source_type(),
            self.entity_type,
            self.name
        )
    }
}

// ---------------------------------------------------------------------------
// InstallTarget
// ---------------------------------------------------------------------------

/// An install target line from the Skillfile: `install <adapter> <scope>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallTarget {
    pub adapter: String,
    pub scope: Scope,
}

impl fmt::Display for InstallTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.adapter, self.scope)
    }
}

// ---------------------------------------------------------------------------
// LockEntry
// ---------------------------------------------------------------------------

/// A lock file entry recording resolved SHA and download URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockEntry {
    pub sha: String,
    pub raw_url: String,
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

/// The fully parsed Skillfile manifest.
#[derive(Debug, Clone, Default)]
pub struct Manifest {
    pub entries: Vec<Entry>,
    pub install_targets: Vec<InstallTarget>,
}

// ---------------------------------------------------------------------------
// InstallOptions
// ---------------------------------------------------------------------------

/// Options controlling install behavior.
#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub dry_run: bool,
    pub overwrite: bool,
}

impl Default for InstallOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            overwrite: true,
        }
    }
}

// ---------------------------------------------------------------------------
// ConflictState
// ---------------------------------------------------------------------------

/// Conflict state persisted in `.skillfile/conflict`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictState {
    pub entry: String,
    pub entity_type: String,
    pub old_sha: String,
    pub new_sha: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_parse_and_display() {
        assert_eq!(Scope::parse("global"), Some(Scope::Global));
        assert_eq!(Scope::parse("local"), Some(Scope::Local));
        assert_eq!(Scope::parse("worldwide"), None);
        assert_eq!(Scope::Global.to_string(), "global");
        assert_eq!(Scope::Local.to_string(), "local");
        assert_eq!(Scope::Global.as_str(), "global");
    }

    #[test]
    fn scope_all_variants() {
        assert_eq!(Scope::ALL.len(), 2);
        assert!(Scope::ALL.contains(&Scope::Global));
        assert!(Scope::ALL.contains(&Scope::Local));
    }

    #[test]
    fn source_fields_typed_accessors() {
        let gh = SourceFields::Github {
            owner_repo: "o/r".into(),
            path_in_repo: "a.md".into(),
            ref_: "main".into(),
        };
        assert_eq!(gh.as_github(), Some(("o/r", "a.md", "main")));
        assert_eq!(gh.as_local(), None);
        assert_eq!(gh.as_url(), None);

        let local = SourceFields::Local {
            path: "test.md".into(),
        };
        assert_eq!(local.as_local(), Some("test.md"));
        assert_eq!(local.as_github(), None);

        let url = SourceFields::Url {
            url: "https://x.com/s.md".into(),
        };
        assert_eq!(url.as_url(), Some("https://x.com/s.md"));
        assert_eq!(url.as_github(), None);
    }

    #[test]
    fn github_entry_source_type() {
        let e = Entry {
            entity_type: "agent".into(),
            name: "test".into(),
            source: SourceFields::Github {
                owner_repo: "o/r".into(),
                path_in_repo: "a.md".into(),
                ref_: "main".into(),
            },
        };
        assert_eq!(e.source_type(), "github");
        assert_eq!(e.entity_type, "agent");
        assert_eq!(e.name, "test");
        assert_eq!(e.owner_repo(), "o/r");
        assert_eq!(e.path_in_repo(), "a.md");
        assert_eq!(e.ref_(), "main");
        assert_eq!(e.local_path(), "");
        assert_eq!(e.url(), "");
    }

    #[test]
    fn github_entry_fields() {
        let e = Entry {
            entity_type: "skill".into(),
            name: "my-skill".into(),
            source: SourceFields::Github {
                owner_repo: "o/r".into(),
                path_in_repo: "skills/s.md".into(),
                ref_: "v1".into(),
            },
        };
        assert_eq!(e.owner_repo(), "o/r");
        assert_eq!(e.path_in_repo(), "skills/s.md");
        assert_eq!(e.ref_(), "v1");
    }

    #[test]
    fn local_entry_fields() {
        let e = Entry {
            entity_type: "skill".into(),
            name: "test".into(),
            source: SourceFields::Local {
                path: "test.md".into(),
            },
        };
        assert_eq!(e.source_type(), "local");
        assert_eq!(e.local_path(), "test.md");
        assert_eq!(e.owner_repo(), "");
        assert_eq!(e.url(), "");
    }

    #[test]
    fn url_entry_fields() {
        let e = Entry {
            entity_type: "skill".into(),
            name: "my-skill".into(),
            source: SourceFields::Url {
                url: "https://example.com/skill.md".into(),
            },
        };
        assert_eq!(e.source_type(), "url");
        assert_eq!(e.url(), "https://example.com/skill.md");
        assert_eq!(e.owner_repo(), "");
    }

    #[test]
    fn entry_display() {
        let e = Entry {
            entity_type: "agent".into(),
            name: "test".into(),
            source: SourceFields::Github {
                owner_repo: "o/r".into(),
                path_in_repo: "a.md".into(),
                ref_: "main".into(),
            },
        };
        assert_eq!(e.to_string(), "github/agent/test");
    }

    #[test]
    fn lock_entry() {
        let le = LockEntry {
            sha: "abc123".into(),
            raw_url: "https://example.com".into(),
        };
        assert_eq!(le.sha, "abc123");
        assert_eq!(le.raw_url, "https://example.com");
    }

    #[test]
    fn install_target_with_scope_enum() {
        let t = InstallTarget {
            adapter: "claude-code".into(),
            scope: Scope::Global,
        };
        assert_eq!(t.adapter, "claude-code");
        assert_eq!(t.scope, Scope::Global);
        assert_eq!(t.to_string(), "claude-code (global)");
    }

    #[test]
    fn manifest_defaults() {
        let m = Manifest::default();
        assert!(m.entries.is_empty());
        assert!(m.install_targets.is_empty());
    }

    #[test]
    fn manifest_with_entries() {
        let e = Entry {
            entity_type: "skill".into(),
            name: "test".into(),
            source: SourceFields::Local {
                path: "test.md".into(),
            },
        };
        let t = InstallTarget {
            adapter: "claude-code".into(),
            scope: Scope::Local,
        };
        let m = Manifest {
            entries: vec![e],
            install_targets: vec![t],
        };
        assert_eq!(m.entries.len(), 1);
        assert_eq!(m.install_targets.len(), 1);
    }

    #[test]
    fn conflict_state_equality() {
        let a = ConflictState {
            entry: "foo".into(),
            entity_type: "agent".into(),
            old_sha: "aaa".into(),
            new_sha: "bbb".into(),
        };
        let b = a.clone();
        assert_eq!(a, b);
    }
}
