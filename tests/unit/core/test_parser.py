from skillfile.core.models import InstallTarget
from skillfile.core.parser import parse_manifest
from tests.helpers import write_manifest

# ---------------------------------------------------------------------------
# Existing entry types (explicit name + ref)
# ---------------------------------------------------------------------------


def test_github_entry_explicit_name_and_ref(tmp_path):
    p = write_manifest(
        tmp_path,
        """\
        github  agent  backend-dev  owner/repo  path/to/agent.md  main
    """,
    )
    m = parse_manifest(p)
    assert len(m.entries) == 1
    e = m.entries[0]
    assert e.source_type == "github"
    assert e.entity_type == "agent"
    assert e.name == "backend-dev"
    assert e.owner_repo == "owner/repo"
    assert e.path_in_repo == "path/to/agent.md"
    assert e.ref == "main"


def test_local_entry_explicit_name(tmp_path):
    p = write_manifest(
        tmp_path,
        """\
        local  skill  git-commit  skills/git/commit.md
    """,
    )
    m = parse_manifest(p)
    assert len(m.entries) == 1
    e = m.entries[0]
    assert e.source_type == "local"
    assert e.entity_type == "skill"
    assert e.name == "git-commit"
    assert e.local_path == "skills/git/commit.md"


def test_url_entry_explicit_name(tmp_path):
    p = write_manifest(
        tmp_path,
        """\
        url  skill  my-skill  https://example.com/skill.md
    """,
    )
    m = parse_manifest(p)
    assert len(m.entries) == 1
    e = m.entries[0]
    assert e.source_type == "url"
    assert e.name == "my-skill"
    assert e.url == "https://example.com/skill.md"


# ---------------------------------------------------------------------------
# Optional name inference
# ---------------------------------------------------------------------------


def test_github_entry_inferred_name(tmp_path):
    p = write_manifest(
        tmp_path,
        """\
        github  agent  owner/repo  path/to/agent.md  main
    """,
    )
    m = parse_manifest(p)
    assert len(m.entries) == 1
    e = m.entries[0]
    assert e.name == "agent"
    assert e.owner_repo == "owner/repo"
    assert e.path_in_repo == "path/to/agent.md"
    assert e.ref == "main"


def test_local_entry_inferred_name_from_path(tmp_path):
    p = write_manifest(
        tmp_path,
        """\
        local  skill  skills/git/commit.md
    """,
    )
    m = parse_manifest(p)
    assert len(m.entries) == 1
    e = m.entries[0]
    assert e.name == "commit"
    assert e.local_path == "skills/git/commit.md"


def test_local_entry_inferred_name_from_md_extension(tmp_path):
    p = write_manifest(
        tmp_path,
        """\
        local  skill  commit.md
    """,
    )
    m = parse_manifest(p)
    assert len(m.entries) == 1
    e = m.entries[0]
    assert e.name == "commit"


def test_url_entry_inferred_name(tmp_path):
    p = write_manifest(
        tmp_path,
        """\
        url  skill  https://example.com/my-skill.md
    """,
    )
    m = parse_manifest(p)
    assert len(m.entries) == 1
    e = m.entries[0]
    assert e.name == "my-skill"
    assert e.url == "https://example.com/my-skill.md"


# ---------------------------------------------------------------------------
# Optional ref (defaults to main)
# ---------------------------------------------------------------------------


def test_github_entry_inferred_name_default_ref(tmp_path):
    p = write_manifest(
        tmp_path,
        """\
        github  agent  owner/repo  path/to/agent.md
    """,
    )
    m = parse_manifest(p)
    assert m.entries[0].ref == "main"


def test_github_entry_explicit_name_default_ref(tmp_path):
    p = write_manifest(
        tmp_path,
        """\
        github  agent  my-agent  owner/repo  path/to/agent.md
    """,
    )
    m = parse_manifest(p)
    assert m.entries[0].ref == "main"


# ---------------------------------------------------------------------------
# Install targets
# ---------------------------------------------------------------------------


def test_install_target_parsed(tmp_path):
    p = write_manifest(
        tmp_path,
        """\
        install  claude-code  global
    """,
    )
    m = parse_manifest(p)
    assert len(m.install_targets) == 1
    assert m.install_targets[0] == InstallTarget(adapter="claude-code", scope="global")


def test_multiple_install_targets(tmp_path):
    p = write_manifest(
        tmp_path,
        """\
        install  claude-code  global
        install  claude-code  local
    """,
    )
    m = parse_manifest(p)
    assert len(m.install_targets) == 2
    assert m.install_targets[0].scope == "global"
    assert m.install_targets[1].scope == "local"


def test_install_targets_not_in_entries(tmp_path):
    p = write_manifest(
        tmp_path,
        """\
        install  claude-code  global
        github  agent  owner/repo  path/to/agent.md
    """,
    )
    m = parse_manifest(p)
    assert len(m.entries) == 1
    assert len(m.install_targets) == 1


# ---------------------------------------------------------------------------
# Comments, blanks, errors
# ---------------------------------------------------------------------------


def test_comments_and_blanks_skipped(tmp_path):
    p = write_manifest(
        tmp_path,
        """\
        # this is a comment

        # another comment
        local  skill  foo  skills/foo.md
    """,
    )
    m = parse_manifest(p)
    assert len(m.entries) == 1


def test_malformed_too_few_fields(tmp_path, capsys):
    p = write_manifest(
        tmp_path,
        """\
        github  agent
    """,
    )
    m = parse_manifest(p)
    assert m.entries == []
    captured = capsys.readouterr()
    assert "warning" in captured.err


def test_unknown_source_type_skipped(tmp_path, capsys):
    p = write_manifest(
        tmp_path,
        """\
        svn  skill  foo  some/path
    """,
    )
    m = parse_manifest(p)
    assert m.entries == []
    captured = capsys.readouterr()
    assert "warning" in captured.err
    assert "svn" in captured.err


# ---------------------------------------------------------------------------
# v0.9.0 — Inline comments (#1)
# ---------------------------------------------------------------------------


def test_inline_comment_stripped(tmp_path):
    """Inline comments must not become fields."""
    p = write_manifest(
        tmp_path,
        """\
        github  agent  owner/repo  agents/foo.md  # my note
    """,
    )
    m = parse_manifest(p)
    assert len(m.entries) == 1
    e = m.entries[0]
    assert e.ref == "main"  # not "#"
    assert e.name == "foo"


def test_inline_comment_on_install_line(tmp_path):
    p = write_manifest(
        tmp_path,
        """\
        install  claude-code  global  # primary target
    """,
    )
    m = parse_manifest(p)
    assert len(m.install_targets) == 1
    assert m.install_targets[0].scope == "global"


def test_inline_comment_after_ref(tmp_path):
    p = write_manifest(
        tmp_path,
        """\
        github  agent  my-agent  owner/repo  agents/foo.md  v1.0  # pinned version
    """,
    )
    m = parse_manifest(p)
    assert m.entries[0].ref == "v1.0"


# ---------------------------------------------------------------------------
# v0.9.0 — Quoted fields (#2)
# ---------------------------------------------------------------------------


def test_quoted_path_with_spaces(tmp_path):
    """Double-quoted fields allow paths with spaces."""
    p = write_manifest(tmp_path, "")
    p.write_text('local  skill  my-skill  "skills/my dir/foo.md"\n')
    m = parse_manifest(p)
    assert len(m.entries) == 1
    assert m.entries[0].local_path == "skills/my dir/foo.md"


def test_quoted_github_path(tmp_path):
    p = write_manifest(tmp_path, "")
    p.write_text('github  skill  owner/repo  "path with spaces/skill.md"\n')
    m = parse_manifest(p)
    assert len(m.entries) == 1
    assert m.entries[0].path_in_repo == "path with spaces/skill.md"


def test_mixed_quoted_and_unquoted(tmp_path):
    """Quoted fields work alongside unquoted fields."""
    p = write_manifest(tmp_path, "")
    p.write_text('github  agent  my-agent  owner/repo  "agents/path with spaces/foo.md"\n')
    m = parse_manifest(p)
    assert len(m.entries) == 1
    assert m.entries[0].name == "my-agent"
    assert m.entries[0].path_in_repo == "agents/path with spaces/foo.md"


def test_unquoted_fields_parse_identically(tmp_path):
    """Unquoted lines must parse identically to before."""
    p = write_manifest(
        tmp_path,
        """\
        github  agent  backend-dev  owner/repo  path/to/agent.md  main
    """,
    )
    m = parse_manifest(p)
    assert m.entries[0].name == "backend-dev"
    assert m.entries[0].ref == "main"


# ---------------------------------------------------------------------------
# v0.9.0 — Name validation (#3)
# ---------------------------------------------------------------------------


def test_valid_entry_name_accepted(tmp_path):
    p = write_manifest(
        tmp_path,
        """\
        local  skill  my-skill_v2.0  skills/foo.md
    """,
    )
    m = parse_manifest(p)
    assert len(m.entries) == 1
    assert m.entries[0].name == "my-skill_v2.0"


def test_invalid_entry_name_rejected(tmp_path, capsys):
    """Names with filesystem-unsafe characters are rejected."""
    p = write_manifest(
        tmp_path,
        """\
        local  skill  "my skill!"  skills/foo.md
    """,
    )
    m = parse_manifest(p)
    assert len(m.entries) == 0
    captured = capsys.readouterr()
    assert "invalid name" in captured.err.lower() or "warning" in captured.err.lower()


def test_inferred_name_validated(tmp_path, capsys):
    """Names inferred from paths are also validated (but normally safe)."""
    p = write_manifest(
        tmp_path,
        """\
        local  skill  skills/foo.md
    """,
    )
    m = parse_manifest(p)
    assert len(m.entries) == 1
    assert m.entries[0].name == "foo"


# ---------------------------------------------------------------------------
# v0.9.0 — Scope validation (#5)
# ---------------------------------------------------------------------------


def test_valid_scope_accepted(tmp_path):
    for scope in ["global", "local"]:
        p = write_manifest(tmp_path, f"install  claude-code  {scope}\n")
        m = parse_manifest(p)
        assert len(m.install_targets) == 1
        assert m.install_targets[0].scope == scope


def test_invalid_scope_rejected(tmp_path, capsys):
    """Unknown scope values must be rejected with a warning."""
    p = write_manifest(
        tmp_path,
        """\
        install  claude-code  worldwide
    """,
    )
    m = parse_manifest(p)
    assert len(m.install_targets) == 0
    captured = capsys.readouterr()
    assert "scope" in captured.err.lower() or "warning" in captured.err.lower()


# ---------------------------------------------------------------------------
# v0.9.0 — UTF-8 BOM handling (#18)
# ---------------------------------------------------------------------------


# ---------------------------------------------------------------------------
# v0.9.0 — Duplicate entry name warning (#9)
# ---------------------------------------------------------------------------


def test_duplicate_entry_name_warns(tmp_path, capsys):
    """Parser warns on duplicate entry names but still includes both."""
    p = write_manifest(
        tmp_path,
        """\
        local  skill  foo  skills/foo.md
        local  agent  foo  agents/foo.md
    """,
    )
    m = parse_manifest(p)
    assert len(m.entries) == 2  # both are included
    captured = capsys.readouterr()
    assert "duplicate" in captured.err.lower()


def test_utf8_bom_handled(tmp_path):
    """Files with UTF-8 BOM must parse correctly."""
    p = tmp_path / "Skillfile"
    p.write_bytes(b"\xef\xbb\xbfinstall  claude-code  global\n")
    m = parse_manifest(p)
    assert len(m.install_targets) == 1
    assert m.install_targets[0] == InstallTarget(adapter="claude-code", scope="global")
