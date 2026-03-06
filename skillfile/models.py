from dataclasses import dataclass


@dataclass
class LockEntry:
    sha: str
    raw_url: str


@dataclass
class Entry:
    source_type: str   # local | github | url
    entity_type: str   # skill | agent
    name: str
    # github
    owner_repo: str = ""
    path_in_repo: str = ""
    ref: str = ""
    # local
    local_path: str = ""
    # url
    url: str = ""
