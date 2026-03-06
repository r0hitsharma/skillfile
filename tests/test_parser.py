import textwrap

from skillfile.models import Entry
from skillfile.parser import parse_manifest


def write_manifest(tmp_path, content):
    p = tmp_path / "Skillfile"
    p.write_text(textwrap.dedent(content))
    return p


def test_github_entry(tmp_path):
    p = write_manifest(tmp_path, """\
        github  agent  backend-dev  owner/repo  path/to/agent.md  main
    """)
    entries = parse_manifest(p)
    assert len(entries) == 1
    e = entries[0]
    assert e.source_type == "github"
    assert e.entity_type == "agent"
    assert e.name == "backend-dev"
    assert e.owner_repo == "owner/repo"
    assert e.path_in_repo == "path/to/agent.md"
    assert e.ref == "main"


def test_local_entry(tmp_path):
    p = write_manifest(tmp_path, """\
        local  skill  git-commit  skills/git/commit.md
    """)
    entries = parse_manifest(p)
    assert len(entries) == 1
    e = entries[0]
    assert e.source_type == "local"
    assert e.entity_type == "skill"
    assert e.name == "git-commit"
    assert e.local_path == "skills/git/commit.md"


def test_url_entry(tmp_path):
    p = write_manifest(tmp_path, """\
        url  skill  my-skill  https://example.com/skill.md
    """)
    entries = parse_manifest(p)
    assert len(entries) == 1
    e = entries[0]
    assert e.source_type == "url"
    assert e.url == "https://example.com/skill.md"


def test_comments_and_blanks_skipped(tmp_path):
    p = write_manifest(tmp_path, """\
        # this is a comment

        # another comment
        local  skill  foo  skills/foo.md
    """)
    entries = parse_manifest(p)
    assert len(entries) == 1


def test_malformed_too_few_fields(tmp_path, capsys):
    p = write_manifest(tmp_path, """\
        github  agent
    """)
    entries = parse_manifest(p)
    assert entries == []
    captured = capsys.readouterr()
    assert "warning" in captured.err


def test_unknown_source_type_skipped(tmp_path, capsys):
    p = write_manifest(tmp_path, """\
        svn  skill  foo  some/path
    """)
    entries = parse_manifest(p)
    assert entries == []
    captured = capsys.readouterr()
    assert "warning" in captured.err
    assert "svn" in captured.err
