"""Unit tests for core/models.py — dataclass construction."""

from skillfile.core.models import Entry, InstallTarget, LockEntry, Manifest


def test_entry_defaults():
    e = Entry("github", "agent", "test")
    assert e.source_type == "github"
    assert e.entity_type == "agent"
    assert e.name == "test"
    assert e.owner_repo == ""
    assert e.path_in_repo == ""
    assert e.ref == ""
    assert e.local_path == ""
    assert e.url == ""


def test_entry_github_fields():
    e = Entry("github", "skill", "my-skill", owner_repo="o/r", path_in_repo="skills/s.md", ref="v1")
    assert e.owner_repo == "o/r"
    assert e.path_in_repo == "skills/s.md"
    assert e.ref == "v1"


def test_lock_entry():
    le = LockEntry(sha="abc123", raw_url="https://example.com")
    assert le.sha == "abc123"
    assert le.raw_url == "https://example.com"


def test_install_target():
    t = InstallTarget(adapter="claude-code", scope="global")
    assert t.adapter == "claude-code"
    assert t.scope == "global"


def test_manifest_defaults():
    m = Manifest()
    assert m.entries == []
    assert m.install_targets == []


def test_manifest_with_entries():
    e = Entry("local", "skill", "test", local_path="test.md")
    t = InstallTarget("claude-code", "local")
    m = Manifest(entries=[e], install_targets=[t])
    assert len(m.entries) == 1
    assert len(m.install_targets) == 1
