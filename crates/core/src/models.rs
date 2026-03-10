use serde::{Deserialize, Serialize};

pub const DEFAULT_REF: &str = "main";

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
    pub fn source_type(&self) -> &str {
        match self {
            SourceFields::Github { .. } => "github",
            SourceFields::Local { .. } => "local",
            SourceFields::Url { .. } => "url",
        }
    }
}

/// A single entry in the Skillfile manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub entity_type: String,
    pub name: String,
    pub source: SourceFields,
}

impl Entry {
    pub fn source_type(&self) -> &str {
        self.source.source_type()
    }

    pub fn owner_repo(&self) -> &str {
        match &self.source {
            SourceFields::Github { owner_repo, .. } => owner_repo,
            _ => "",
        }
    }

    pub fn path_in_repo(&self) -> &str {
        match &self.source {
            SourceFields::Github { path_in_repo, .. } => path_in_repo,
            _ => "",
        }
    }

    pub fn ref_(&self) -> &str {
        match &self.source {
            SourceFields::Github { ref_, .. } => ref_,
            _ => "",
        }
    }

    pub fn local_path(&self) -> &str {
        match &self.source {
            SourceFields::Local { path } => path,
            _ => "",
        }
    }

    pub fn url(&self) -> &str {
        match &self.source {
            SourceFields::Url { url } => url,
            _ => "",
        }
    }
}

/// An install target line from the Skillfile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallTarget {
    pub adapter: String,
    pub scope: String,
}

/// A lock file entry recording resolved SHA and download URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockEntry {
    pub sha: String,
    pub raw_url: String,
}

/// The fully parsed Skillfile manifest.
#[derive(Debug, Clone, Default)]
pub struct Manifest {
    pub entries: Vec<Entry>,
    pub install_targets: Vec<InstallTarget>,
}

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

/// Conflict state persisted in `.skillfile/conflict`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictState {
    pub entry: String,
    pub entity_type: String,
    pub old_sha: String,
    pub new_sha: String,
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // Non-applicable accessors return ""
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
    fn lock_entry() {
        let le = LockEntry {
            sha: "abc123".into(),
            raw_url: "https://example.com".into(),
        };
        assert_eq!(le.sha, "abc123");
        assert_eq!(le.raw_url, "https://example.com");
    }

    #[test]
    fn install_target() {
        let t = InstallTarget {
            adapter: "claude-code".into(),
            scope: "global".into(),
        };
        assert_eq!(t.adapter, "claude-code");
        assert_eq!(t.scope, "global");
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
            scope: "local".into(),
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
