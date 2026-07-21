"""Generate synthetic Claude session JSONL fixtures for fork tree testing.

Run from the repo root:
    python tests/scripts/gen-claude-fixtures.py

Or from any directory with:
    python path/to/gen-claude-fixtures.py

Output: tests/fixtures/synthetic-claude/*.jsonl
"""

import hashlib
import json
import os

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
FIXTURE_DIR = os.path.join(SCRIPT_DIR, "..", "fixtures", "synthetic-claude")


def make_uuid(seed):
    """Generate a deterministic UUID (RFC 4122 format) from a seed string."""
    h = hashlib.md5(seed.encode()).hexdigest()
    return f"{h[0:8]}-{h[8:12]}-{h[12:16]}-{h[16:20]}-{h[20:32]}"


FIXTURES = [
    # ── A: root, 4 events ───────────────────────────────────────────
    {
        "name": "A--init--config--write-api--test.jsonl",
        "id": "synth-u-a",
        "uuid_seeds": ["A1", "A2", "A3", "A4"],
        "texts": [
            "init project",
            "configure database",
            "write REST API",
            "test endpoints",
        ],
    },
    # ── B: child of A, fork=2 (first 2 uuids match A) ──────────────
    {
        "name": "B-of-A--init--config--add-auth--test-auth.jsonl",
        "id": "synth-u-b",
        "uuid_seeds": ["A1", "A2", "B3", "B4"],
        "texts": [
            "init project",
            "configure database",
            "add authentication",
            "test auth",
        ],
    },
    # ── C: child of A, fork=3 (first 3 uuids match A) ──────────────
    {
        "name": "C-of-A--init--config--write-api--add-caching.jsonl",
        "id": "synth-u-c",
        "uuid_seeds": ["A1", "A2", "A3", "C4"],
        "texts": [
            "init project",
            "configure database",
            "write REST API",
            "add redis caching",
        ],
    },
    # ── D: root, 3 events ───────────────────────────────────────────
    {
        "name": "D--setup--ci--deploy.jsonl",
        "id": "synth-u-d",
        "uuid_seeds": ["D1", "D2", "D3"],
        "texts": [
            "setup project",
            "add CI pipeline",
            "deploy to prod",
        ],
    },
    # ── E: child of D, fork=1 (first uuid matches D) ───────────────
    # NOTE: E has 3 events (same uuid_chain length as D) so Path B's
    # sort-by-length-ascending correctly identifies D as the parent.
    # (Child must be >= parent chain length for the LCP heuristic.)
    {
        "name": "E-of-D--setup--docker--validate.jsonl",
        "id": "synth-u-e",
        "uuid_seeds": ["D1", "E2", "E3"],
        "texts": [
            "setup project",
            "setup Docker",
            "validate compose",
        ],
    },
    # ── F: standalone root, 2 events ───────────────────────────────
    {
        "name": "F--readme--license.jsonl",
        "id": "synth-u-f",
        "uuid_seeds": ["F1", "F2"],
        "texts": [
            "update readme",
            "add license",
        ],
    },
    # ── G: standalone root, 2 events ───────────────────────────────
    {
        "name": "G--debug--fix.jsonl",
        "id": "synth-u-g",
        "uuid_seeds": ["G1", "G2"],
        "texts": [
            "debug crash",
            "fix bug",
        ],
    },
    # ── H: standalone root, 1 event (greeting filtered from hash) ──
    {
        "name": "H--hello.jsonl",
        "id": "synth-u-h",
        "uuid_seeds": ["H1"],
        "texts": [
            "hello",
        ],
    },
    # ── S1: subagent, 1 event (filename starts with "agent-") ──────
    {
        "name": "agent-s1-sub--explore.jsonl",
        "id": "synth-u-s1",
        "uuid_seeds": ["S1"],
        "texts": ["explore codebase"],
        "subagent": True,
    },
    # ── S2: subagent, 1 event (filename starts with "agent-") ──────
    {
        "name": "agent-s2-sub--check-deps.jsonl",
        "id": "synth-u-s2",
        "uuid_seeds": ["S2"],
        "texts": ["check deps"],
        "subagent": True,
    },
]


def write_fixture(fixture):
    """Write one .jsonl fixture file."""
    path = os.path.join(FIXTURE_DIR, fixture["name"])
    lines = []

    # ── Head line: session metadata ────────────────────────────────
    # Claude format uses sessionId + cwd on the first line (NOT a
    # session_meta payload like Codex).
    lines.append(
        json.dumps(
            {
                "sessionId": fixture["id"],
                "cwd": "D:\\project",
                "timestamp": "2026-07-14T12:00:00Z",
            },
            ensure_ascii=False,
        )
    )

    # ── User event lines with deterministic UUIDs ──────────────────
    # parentUuid chains each message to the previous one for realism,
    # though the fork tree code does not read this field.
    uuids = [make_uuid(seed) for seed in fixture["uuid_seeds"]]
    prev_uuid = None
    for i, (uuid, text) in enumerate(zip(uuids, fixture["texts"])):
        event = {
            "type": "user",
            "uuid": uuid,
            "message": {
                "role": "user",
                "content": [{"type": "text", "text": text}],
            },
            "timestamp": f"2026-07-14T12:{i + 1:02d}:00Z",
        }
        if prev_uuid:
            event["parentUuid"] = prev_uuid
        lines.append(json.dumps(event, ensure_ascii=False))
        prev_uuid = uuid

    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(lines))
        f.write("\n")

    print(f"  {fixture['name']} ({len(fixture['texts'])} events)")


def main():
    os.makedirs(FIXTURE_DIR, exist_ok=True)
    print(f"Generating synthetic Claude fixtures in {FIXTURE_DIR}...\n")
    for fixture in FIXTURES:
        write_fixture(fixture)
    total_bytes = sum(
        os.path.getsize(os.path.join(FIXTURE_DIR, f["name"])) for f in FIXTURES
    )
    print(f"\nDone: {len(FIXTURES)} files, {total_bytes / 1024:.1f} KB")


if __name__ == "__main__":
    main()
