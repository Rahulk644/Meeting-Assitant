#!/usr/bin/env python3
"""Record human macOS acceptance evidence without committing private session data."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import subprocess
import sys
import uuid
from datetime import datetime, timezone
from pathlib import Path


HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
DEFAULT_OUT = Path(
    os.environ.get("ASSISTANT_ACCEPTANCE_CACHE", "~/.cache/my-assistant-acceptance")
).expanduser()
RESULTS = {"pass", "fail", "blocked", "not-applicable"}
PSEUDONYM = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{1,63}\Z")


def run_text(command: list[str]) -> str:
    result = subprocess.run(command, capture_output=True, text=True)
    return result.stdout.strip() if result.returncode == 0 else ""


def cases() -> dict:
    data = json.loads((HERE / "cases.json").read_text())
    if data.get("schemaVersion") != 1:
        raise ValueError("unsupported cases schema")
    return data["suites"]


def system_metadata() -> dict:
    tauri = json.loads((ROOT / "desktop/src-tauri/tauri.conf.json").read_text())
    git_status = run_text(["git", "-C", str(ROOT), "status", "--short"])
    return {
        "appVersion": tauri["version"],
        "gitCommit": run_text(["git", "-C", str(ROOT), "rev-parse", "HEAD"]),
        "gitDirty": bool(git_status),
        "macOS": run_text(["sw_vers", "-productVersion"]),
        "build": run_text(["sw_vers", "-buildVersion"]),
        "machine": platform.machine(),
        "hardware": run_text(["sysctl", "-n", "hw.model"]),
    }


def save_session(path: Path, session: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(session, ensure_ascii=False, indent=2) + "\n")
    temporary.replace(path)


def evidence_metadata(paths: list[str]) -> list[dict]:
    metadata: list[dict] = []
    for value in paths:
        path = Path(value)
        if not path.is_file():
            continue
        digest = hashlib.sha256()
        with path.open("rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
        metadata.append(
            {"path": str(path), "bytes": path.stat().st_size, "sha256": digest.hexdigest()}
        )
    return metadata


def new_session(args: argparse.Namespace) -> None:
    suite = cases()[args.suite]
    if not PSEUDONYM.fullmatch(args.operator_id):
        raise ValueError("--operator-id must be a pseudonym using letters, numbers, dot, underscore, or hyphen")
    if args.suite == "two-person-meeting":
        if not args.platform:
            raise ValueError("--platform is required for two-person-meeting")
        if args.platform not in suite["platforms"]:
            raise ValueError(f"unsupported platform {args.platform!r}")
    elif args.platform:
        raise ValueError("--platform applies only to two-person-meeting")
    session_id = f"{datetime.now().strftime('%Y%m%d-%H%M%S')}-{uuid.uuid4().hex[:8]}"
    output = (
        args.out.expanduser().resolve()
        if args.out
        else DEFAULT_OUT / args.suite / f"{session_id}.json"
    )
    session = {
        "schemaVersion": 1,
        "sessionId": session_id,
        "suite": args.suite,
        "platform": args.platform or None,
        "operatorId": args.operator_id,
        "createdAt": datetime.now(timezone.utc).isoformat(),
        "requires": suite["requires"],
        "system": system_metadata(),
        "platformVersion": args.platform_version or None,
        "notes": args.notes,
        "results": {
            case["id"]: {
                "result": "pending",
                "evidence": [],
                "evidenceMetadata": [],
                "notes": "",
                "recordedAt": None,
            }
            for case in suite["cases"]
        },
    }
    save_session(output, session)
    print(output)


def load_session(path: Path) -> dict:
    session = json.loads(path.read_text())
    if session.get("schemaVersion") != 1 or session.get("suite") not in cases():
        raise ValueError("invalid acceptance session")
    return session


def record(args: argparse.Namespace) -> None:
    path = args.session.expanduser().resolve()
    session = load_session(path)
    suite_cases = {case["id"]: case for case in cases()[session["suite"]]["cases"]}
    if args.case not in suite_cases:
        raise ValueError(f"unknown case {args.case!r}")
    if args.result not in RESULTS:
        raise ValueError(f"unsupported result {args.result!r}")
    evidence = [str(Path(value).expanduser().resolve()) for value in args.evidence]
    if args.result == "pass" and not evidence:
        raise ValueError("a pass requires at least one screenshot, log, recording, or observation-note path")
    if args.result == "pass" and not any(Path(value).is_file() for value in evidence):
        raise ValueError("a pass requires at least one evidence file that exists")
    session["results"][args.case] = {
        "result": args.result,
        "evidence": evidence,
        "evidenceMetadata": evidence_metadata(evidence),
        "notes": args.notes,
        "recordedAt": datetime.now(timezone.utc).isoformat(),
    }
    save_session(path, session)
    print(f"{args.case}: {args.result}")


def changed_pass_evidence(session: dict) -> list[str]:
    changed: list[str] = []
    for case_id, result in session["results"].items():
        if result["result"] != "pass":
            continue
        pinned = result.get("evidenceMetadata", [])
        current = evidence_metadata(result.get("evidence", []))
        if not pinned or current != pinned:
            changed.append(case_id)
    return changed


def summary(args: argparse.Namespace) -> int:
    session = load_session(args.session.expanduser().resolve())
    counts: dict[str, int] = {}
    for result in session["results"].values():
        outcome = result["result"]
        counts[outcome] = counts.get(outcome, 0) + 1
    changed_evidence = changed_pass_evidence(session)
    print(
        json.dumps(
            {
                "session": session["sessionId"],
                "counts": counts,
                "changedOrMissingPassEvidence": changed_evidence,
            },
            indent=2,
        )
    )
    complete = not changed_evidence and not any(
        outcome in counts for outcome in ("pending", "fail", "blocked")
    )
    return 0 if complete else 1


def walk(args: argparse.Namespace) -> int:
    path = args.session.expanduser().resolve()
    session = load_session(path)
    suite = cases()[session["suite"]]
    choices = {"p": "pass", "f": "fail", "b": "blocked", "n": "not-applicable"}
    for case in suite["cases"]:
        current = session["results"][case["id"]]["result"]
        if current != "pending" and not args.all:
            continue
        print(f"\n{case['id']}  {case['name']}\nDo: {case['steps']}\nPass: {case['expected']}")
        while True:
            choice = input("Result [p]ass [f]ail [b]locked [n]/a [s]kip: ").strip().lower()
            if choice == "s":
                break
            if choice in choices:
                outcome = choices[choice]
                observation = input("Observed (required for pass): ").strip()
                if outcome == "pass" and not observation:
                    print("A pass needs a concrete observation.")
                    continue
                evidence: list[str] = []
                if observation:
                    evidence_dir = path.parent / f"{path.stem}-evidence"
                    evidence_dir.mkdir(parents=True, exist_ok=True)
                    observation_path = evidence_dir / f"{case['id']}-observation.txt"
                    observation_path.write_text(
                        f"Case: {case['id']} {case['name']}\n"
                        f"Expected: {case['expected']}\n"
                        f"Observed: {observation}\n"
                        f"Recorded: {datetime.now(timezone.utc).isoformat()}\n"
                    )
                    evidence.append(str(observation_path))
                session["results"][case["id"]] = {
                    "result": outcome,
                    "evidence": evidence,
                    "evidenceMetadata": evidence_metadata(evidence),
                    "notes": observation,
                    "recordedAt": datetime.now(timezone.utc).isoformat(),
                }
                save_session(path, session)
                break
            print("Choose p, f, b, n, or s.")
    return summary(argparse.Namespace(session=path))


def list_cases(args: argparse.Namespace) -> None:
    suite = cases()[args.suite]
    print(suite["requires"])
    if "platforms" in suite:
        print("Platforms: " + ", ".join(suite["platforms"]))
    for case in suite["cases"]:
        print(f"\n{case['id']}  {case['name']}\n  Do: {case['steps']}\n  Pass: {case['expected']}")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    listing = commands.add_parser("list")
    listing.add_argument("--suite", choices=cases(), required=True)
    listing.set_defaults(func=list_cases)
    new = commands.add_parser("new")
    new.add_argument("--suite", choices=cases(), required=True)
    new.add_argument("--platform")
    new.add_argument("--platform-version")
    new.add_argument("--operator-id", required=True, help="Pseudonym, not a personal name or email")
    new.add_argument("--notes", default="")
    new.add_argument("--out", type=Path)
    new.set_defaults(func=new_session)
    recording = commands.add_parser("record")
    recording.add_argument("--session", type=Path, required=True)
    recording.add_argument("--case", required=True)
    recording.add_argument("--result", choices=sorted(RESULTS), required=True)
    recording.add_argument("--evidence", action="append", default=[])
    recording.add_argument("--notes", default="")
    recording.set_defaults(func=record)
    guided = commands.add_parser("walk", help="Guided, resumable human acceptance walkthrough")
    guided.add_argument("--session", type=Path, required=True)
    guided.add_argument("--all", action="store_true", help="Revisit completed cases too")
    guided.set_defaults(func=walk)
    report = commands.add_parser("summary")
    report.add_argument("--session", type=Path, required=True)
    report.set_defaults(func=summary)
    return parser


def main() -> int:
    args = build_parser().parse_args()
    result = args.func(args)
    return result if isinstance(result, int) else 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1)
