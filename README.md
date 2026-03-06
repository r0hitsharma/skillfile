# skillfile

Declarative manager for AI skills and agents - the Brewfile for your AI tooling.

## What it is

AI frameworks like Claude Code and Codex consume markdown files that define skills, agents, and commands. There is no standard way to manage these across tools or machines. You end up copying files by hand, losing track of upstream versions, and having no reproducibility across machines.

`skillfile` fixes this. You declare what you want in a manifest (`Skillfile`), run `sync`, and your skills and agents are fetched, vendored into your repo, and pinned to an exact commit SHA. Everything is in git. Setup is fully reproducible.

It is not a framework. It does not run agents. It only manages the markdown files that frameworks consume.

## Status

**v0.2.0** - sync and lock file work. Adapter layer (`install`) coming in v0.3.

For now, after `sync` you place files manually from `vendor/` into your tool's expected directory. The `install` command will automate this.

## Usage

```
python3 -m skillfile sync              # fetch all entries, write Skillfile.lock
python3 -m skillfile sync --dry-run   # show what would change
python3 -m skillfile sync --update    # re-resolve all refs and update the lock
python3 -m skillfile sync --entry NAME

python3 -m skillfile status           # show locked/unlocked state of all entries
python3 -m skillfile status --check-upstream  # compare locked SHAs against upstream
```

## Skillfile format

```
# <source>  <type>  <name>  [source fields...]

# GitHub
github  agent  backend-developer  VoltAgent/awesome-claude-code-subagents  categories/01-core-development/backend-developer.md  main

# Local
local  skill  git-commit  skills/git/commit.md

# Direct URL
url  skill  my-skill  https://example.com/skill.md
```

Line-oriented, space-delimited, human-editable. No YAML, no TOML.

## Directory layout

```
Skillfile                  ← manifest
Skillfile.lock             ← pinned SHAs, committed to git
vendor/
  agents/
    backend-developer/
      backend-developer.md ← vendored file
      .meta                ← source URL and SHA
skills/                    ← your own local skill definitions
agents/                    ← your own local agent definitions
```

## Requirements

Python 3.10+, stdlib only. No dependencies.
