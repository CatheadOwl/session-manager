"""Generate synthetic Codex session JSONL fixtures for fork tree testing.

Run from the repo root:
    python tests/scripts/gen-codex-fixtures.py

Or from any directory with:
    python path/to/gen-codex-fixtures.py

Output: tests/fixtures/synthetic-codex/*.jsonl
"""

import json
import os

# Target directory (relative to this script)
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
FIXTURE_DIR = os.path.join(
    SCRIPT_DIR, "..", "fixtures", "synthetic-codex"
)

FIXTURES = [
    # (filename, session_id, user_messages, forked_from_id?, is_subagent?)
    {
        "name": "A--hello--check-types--fix-bug--deploy.jsonl",
        "id": "synth-a-0000",
        "msgs": ["hello", "check the types in api.rs", "fix the type error", "deploy to staging"],
    },
    {
        "name": "B-of-A--hello--check-types--add-tests--refactor--docs.jsonl",
        "id": "synth-b-1111",
        "msgs": ["hello", "check the types in api.rs", "add unit tests", "refactor the module", "update docs"],
        "forked_from": "synth-a-0000",
    },
    {
        "name": "C--init--config-db--write-api--test.jsonl",
        "id": "synth-c-2222",
        "msgs": ["init project", "configure database", "write REST API", "test endpoints"],
    },
    {
        "name": "D-of-C--init--config-db--add-auth.jsonl",
        "id": "synth-d-3333",
        "msgs": ["init project", "configure database", "add authentication middleware"],
        "forked_from": "synth-c-2222",
    },
    {
        "name": "E-of-C--init--config-db--write-api--add-caching.jsonl",
        "id": "synth-e-4444",
        "msgs": ["init project", "configure database", "write REST API", "add redis caching"],
        "forked_from": "synth-c-2222",
    },
    {
        "name": "F--setup--first-commit.jsonl",
        "id": "synth-f-5555",
        "msgs": ["setup project scaffold", "make initial commit"],
    },
    {
        "name": "G-of-F--setup--add-ci--add-docker.jsonl",
        "id": "synth-g-6666",
        "msgs": ["setup project scaffold", "add CI pipeline", "add Dockerfile"],
        "forked_from": "synth-f-5555",
    },
    {
        "name": "H--readme--license.jsonl",
        "id": "synth-h-7777",
        "msgs": ["write README", "choose a license"],
    },
    {
        "name": "I-of-H--readme--contributing-guide.jsonl",
        "id": "synth-i-8888",
        "msgs": ["write README", "draft contributing guide"],
        "forked_from": "synth-h-7777",
    },
    {
        "name": "K--fix-typo--done.jsonl",
        "id": "synth-k-9999",
        "msgs": ["fix the typo in footer", "done"],
    },
    {
        "name": "L--upgrade-deps.jsonl",
        "id": "synth-l-aaaa",
        "msgs": ["upgrade all npm dependencies"],
    },
    {
        "name": "M--debug-crash.jsonl",
        "id": "synth-m-bbbb",
        "msgs": ["debug the crash on startup"],
    },
    {
        "name": "N-orphan--refactor--review.jsonl",
        "id": "synth-n-cccc",
        "msgs": ["refactor the auth module", "review all changes"],
        "forked_from": "nonexistent-parent",
    },
    {
        "name": "S1-subagent--explore-codebase.jsonl",
        "id": "synth-s1-sub",
        "msgs": ["explore the codebase structure"],
        "subagent": True,
    },
    {
        "name": "S2-subagent--check-types.jsonl",
        "id": "synth-s2-sub",
        "msgs": ["check for type errors"],
        "subagent": True,
    },
]


def make_meta_payload(session_id, forked_from=None, is_subagent=False):
    """Build the session_meta payload dict."""
    payload = {
        "id": session_id,
        "cwd": "D:\\Document\\Projects\\test-project",
        "originator": "Codex Desktop",
        "cli_version": "0.144.2",
        "source": "vscode",
        "model_provider": "custom",
    }
    if forked_from:
        payload["forked_from_id"] = forked_from
    if is_subagent:
        payload["source"] = {
            "subagent": {
                "thread_spawn": {
                    "parent_thread_id": "parent-id",
                    "depth": 1,
                    "agent_role": "explorer",
                }
            }
        }
    return payload


def write_fixture(fixture):
    """Write one .jsonl fixture file."""
    path = os.path.join(FIXTURE_DIR, fixture["name"])
    lines = []

    # Session meta (head line)
    lines.append(
        json.dumps(
            {
                "timestamp": "2026-07-14T12:00:00Z",
                "type": "session_meta",
                "payload": make_meta_payload(
                    fixture["id"],
                    forked_from=fixture.get("forked_from"),
                    is_subagent=fixture.get("subagent", False),
                ),
            },
            ensure_ascii=False,
        )
    )

    # User event lines
    for i, msg in enumerate(fixture["msgs"]):
        lines.append(
            json.dumps(
                {
                    "timestamp": f"2026-07-14T12:0{i+1}:00Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "user",
                        "content": [{"type": "input_text", "text": msg}],
                    },
                },
                ensure_ascii=False,
            )
        )

    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(lines))
        f.write("\n")

    print(f"  {fixture['name']} ({len(fixture['msgs'])} events)")


def main():
    os.makedirs(FIXTURE_DIR, exist_ok=True)
    print(f"Generating synthetic Codex fixtures in {FIXTURE_DIR}...\n")
    for fixture in FIXTURES:
        write_fixture(fixture)
    total_kb = (
        sum(
            os.path.getsize(os.path.join(FIXTURE_DIR, f["name"]))
            for f in FIXTURES
        )
        / 1024
    )
    print(f"\nDone: {len(FIXTURES)} files, {total_kb:.1f} KB")


if __name__ == "__main__":
    main()
