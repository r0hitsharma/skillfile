import json

from skillfile.core.lock import lock_key, read_lock, write_lock
from skillfile.core.models import Entry, LockEntry


def make_github_entry(name="test"):
    return Entry(
        source_type="github",
        entity_type="agent",
        name=name,
        owner_repo="owner/repo",
        path_in_repo="agent.md",
        ref="main",
    )


def test_lock_key_format():
    e = make_github_entry("my-agent")
    assert lock_key(e) == "github/agent/my-agent"


def test_write_lock_valid_json(tmp_path):
    locked = {"github/agent/test": LockEntry(sha="abc123", raw_url="https://example.com/file.md")}
    write_lock(tmp_path, locked)
    content = (tmp_path / "Skillfile.lock").read_text()
    data = json.loads(content)
    assert data == {"github/agent/test": {"sha": "abc123", "raw_url": "https://example.com/file.md"}}


def test_read_lock_missing_file(tmp_path):
    result = read_lock(tmp_path)
    assert result == {}


def test_roundtrip(tmp_path):
    locked = {
        "github/agent/foo": LockEntry(sha="deadbeef", raw_url="https://example.com/foo.md"),
        "github/skill/bar": LockEntry(sha="cafebabe", raw_url="https://example.com/bar.md"),
    }
    write_lock(tmp_path, locked)
    result = read_lock(tmp_path)
    assert result == locked
