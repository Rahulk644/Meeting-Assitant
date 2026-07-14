#!/usr/bin/env python3
"""Create and validate private, human-reviewed speech evaluation corpora."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import random
import re
import unicodedata
from datetime import datetime, timezone
from pathlib import Path


DEFAULT_CACHE = Path(os.environ.get("ASSISTANT_BENCH_CACHE", "~/.cache/my-assistant-bench")).expanduser()
REAL_CONDITIONS = {"clean", "cafe", "wind", "traffic", "fan", "echo", "overlap", "silence"}
PSEUDONYM = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{1,63}\Z")


def hash_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(8 * 1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_jsonl(path: Path) -> list[dict]:
    records: list[dict] = []
    with path.open(encoding="utf-8") as handle:
        for number, line in enumerate(handle, start=1):
            if line.strip():
                try:
                    records.append(json.loads(line))
                except json.JSONDecodeError as error:
                    raise ValueError(f"{path}:{number}: {error}") from error
    return records


def load_manifest(path: Path) -> tuple[dict, list[dict]]:
    records = load_jsonl(path)
    headers = [record for record in records if record.get("type") == "corpus_header"]
    samples = [record for record in records if record.get("type") != "corpus_header"]
    if len(headers) != 1 or not samples:
        raise ValueError("source manifest needs one corpus_header and at least one sample")
    for sample in samples:
        audio = (path.parent / sample["audio"]).resolve()
        if not audio.is_file():
            raise ValueError(f"missing source audio for {sample.get('id')}: {audio}")
        sample["audio"] = str(audio)
    return headers[0], samples


def is_roman_reference(text: str) -> bool:
    if not text.strip():
        return False
    for character in unicodedata.normalize("NFC", text):
        category = unicodedata.category(character)
        name = unicodedata.name(character, "")
        if category.startswith("L") and "LATIN" not in name:
            return False
        if category.startswith("M") and not name.startswith("COMBINING"):
            return False
        if category.startswith("C") and character not in {"\n", "\t"}:
            return False
    return True


def annotation_template(task: dict) -> dict:
    return {
        "id": task["id"],
        "romanReference": "",
        "transcriberId": "",
        "reviewerId": "",
        "status": "pending",
        "confidence": "",
        "notes": "",
    }


def annotation_html(tasks: list[dict], pack_id: str) -> str:
    public_tasks = [
        {"id": task["id"], "audio": Path(task["audio"]).resolve().as_uri()} for task in tasks
    ]
    payload = json.dumps(public_tasks, ensure_ascii=False).replace("</", "<\\/")
    return f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Human Roman transcript review</title>
<style>
body{{font:15px -apple-system,BlinkMacSystemFont,sans-serif;margin:0;background:#f5f5f7;color:#18181b}}
main{{max-width:760px;margin:32px auto;padding:28px;background:white;border-radius:18px;box-shadow:0 10px 40px #0001}}
.row{{display:flex;gap:12px;align-items:center;flex-wrap:wrap}} input,textarea,select,button{{font:inherit}}
input,textarea,select{{border:1px solid #d4d4d8;border-radius:10px;padding:10px}} textarea{{width:100%;min-height:130px;box-sizing:border-box}}
audio{{width:100%;margin:18px 0}} button{{border:0;border-radius:10px;padding:10px 16px;cursor:pointer}} .primary{{background:#2563eb;color:white}}
.muted{{color:#71717a}} .spacer{{flex:1}} #progress{{font-variant-numeric:tabular-nums}}</style></head>
<body><main><h1>Roman Hinglish annotation</h1>
<p class="muted">Listen blind. Write exactly what was spoken in natural Latin-script Hinglish. Do not use ASR, translation, or machine transliteration. A different person must review every approved item.</p>
<div class="row"><label>Annotator ID <input id="person" autocomplete="off"></label><label>Role <select id="role"><option value="transcriber">Transcriber</option><option value="reviewer">Reviewer</option></select></label><span class="spacer"></span><strong id="progress"></strong></div>
<h2 id="task"></h2><audio id="audio" controls preload="metadata"></audio>
<label>Roman reference<textarea id="text" spellcheck="false"></textarea></label>
<div class="row"><label>Status <select id="status"><option>pending</option><option>transcribed</option><option>approved</option><option>reject</option></select></label><label>Confidence <select id="confidence"><option></option><option>high</option><option>medium</option><option>low</option></select></label></div>
<p><label>Notes <input id="notes" style="width:100%;box-sizing:border-box"></label></p>
<div class="row"><button id="prev">Previous</button><button class="primary" id="save">Save & next</button><span class="spacer"></span><label>Import <input id="import" type="file" accept=".jsonl"></label><button id="export">Export JSONL</button></div>
<p class="muted">Pack: {pack_id}. Progress is stored only in this browser profile; export often.</p></main>
<script>const TASKS={payload},KEY="assistant-corpus-{pack_id}",state=JSON.parse(localStorage.getItem(KEY)||"{{}}"),$=s=>document.querySelector(s);let i=0;
function blank(id){{return {{id,romanReference:"",transcriberId:"",reviewerId:"",status:"pending",confidence:"",notes:""}}}}function cur(){{return state[TASKS[i].id]||blank(TASKS[i].id)}}
function paint(){{const a=cur();$("#task").textContent=TASKS[i].id;$("#audio").src=TASKS[i].audio;$("#text").value=a.romanReference;$("#status").value=a.status;$("#confidence").value=a.confidence;$("#notes").value=a.notes;$("#progress").textContent=`${{i+1}} / ${{TASKS.length}}`;}}
function save(){{const a=cur(),person=$("#person").value.trim(),role=$("#role").value;if(!person)throw Error("Enter a pseudonymous annotator ID");a.romanReference=$("#text").value.trim();a.status=$("#status").value;a.confidence=$("#confidence").value;a.notes=$("#notes").value.trim();a[role+"Id"]=person;if(a.transcriberId&&a.reviewerId&&a.transcriberId===a.reviewerId)throw Error("Reviewer must be a different person");state[a.id]=a;localStorage.setItem(KEY,JSON.stringify(state));}}
$("#save").onclick=()=>{{try{{save();i=Math.min(i+1,TASKS.length-1);paint()}}catch(e){{alert(e.message)}}}};$("#prev").onclick=()=>{{i=Math.max(0,i-1);paint()}};
$("#export").onclick=()=>{{try{{save()}}catch(e){{alert(e.message);return}}const lines=TASKS.map(t=>JSON.stringify(state[t.id]||blank(t.id))).join("\\n")+"\\n",a=document.createElement("a");a.href=URL.createObjectURL(new Blob([lines],{{type:"application/x-ndjson"}}));a.download="annotations.jsonl";a.click();URL.revokeObjectURL(a.href)}};
$("#import").onchange=async e=>{{const text=await e.target.files[0].text();for(const line of text.split(/\\n/))if(line.trim()){{const a=JSON.parse(line);if(TASKS.some(t=>t.id===a.id))state[a.id]=a}}localStorage.setItem(KEY,JSON.stringify(state));paint()}};paint();</script></body></html>"""


def write_pack(out_dir: Path, tasks: list[dict], metadata: dict) -> None:
    if out_dir.exists() and any(out_dir.iterdir()):
        raise ValueError(f"output directory is not empty: {out_dir}")
    out_dir.mkdir(parents=True, exist_ok=True)
    pack_id = hashlib.sha256(
        "\n".join(task["id"] for task in tasks).encode()
    ).hexdigest()[:16]
    metadata = {**metadata, "schemaVersion": 1, "packId": pack_id, "samples": len(tasks)}
    (out_dir / "pack.json").write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n")
    (out_dir / "tasks.json").write_text(json.dumps(tasks, ensure_ascii=False, indent=2) + "\n")
    with (out_dir / "annotations-template.jsonl").open("w", encoding="utf-8") as handle:
        for task in tasks:
            handle.write(json.dumps(annotation_template(task), ensure_ascii=False) + "\n")
    (out_dir / "annotate.html").write_text(annotation_html(tasks, pack_id))


def create_roman_pack(args: argparse.Namespace) -> None:
    source = args.source_manifest.expanduser().resolve()
    header, samples = load_manifest(source)
    if args.count > len(samples):
        raise ValueError(f"requested {args.count} samples from a {len(samples)}-sample manifest")
    selected = samples.copy()
    if args.count < len(samples):
        selected = random.Random(args.seed).sample(samples, args.count)
        selected.sort(key=lambda item: item["id"])
    tasks = [
        {"id": sample["id"], "audio": sample["audio"], "sourceRow": sample.get("sourceRow")}
        for sample in selected
    ]
    write_pack(
        args.out_dir.expanduser().resolve(),
        tasks,
        {
            "kind": "human-roman",
            "sourceManifest": str(source),
            "sourceManifestSha256": hash_file(source),
            "sourceDataset": header.get("dataset"),
            "sourceRevision": header.get("revision"),
            "sourceLicense": header.get("license"),
            "selection": {"seed": args.seed, "count": len(tasks)},
        },
    )


def create_real_pack(args: argparse.Namespace) -> None:
    captures = args.captures.expanduser().resolve()
    records = load_jsonl(captures)
    if not records:
        raise ValueError("capture manifest is empty")
    tasks: list[dict] = []
    for record in records:
        if record.get("acousticOrigin") != "field-recorded":
            raise ValueError(f"{record.get('id')}: real-noise pack forbids playback/synthetic audio")
        if record.get("condition") not in REAL_CONDITIONS:
            raise ValueError(f"{record.get('id')}: unsupported real-noise condition")
        audio = (captures.parent / record["audio"]).resolve()
        if not audio.is_file() or hash_file(audio) != record.get("audioSha256"):
            raise ValueError(f"{record.get('id')}: missing or changed audio")
        tasks.append({**record, "audio": str(audio)})
    write_pack(
        args.out_dir.expanduser().resolve(),
        tasks,
        {
            "kind": "real-noise",
            "sourceCaptureManifest": str(captures),
            "sourceCaptureManifestSha256": hash_file(captures),
            "conditions": sorted({task["condition"] for task in tasks}),
        },
    )


def finalize(args: argparse.Namespace) -> None:
    pack_dir = args.pack.expanduser().resolve()
    metadata = json.loads((pack_dir / "pack.json").read_text())
    tasks = {task["id"]: task for task in json.loads((pack_dir / "tasks.json").read_text())}
    annotations_path = args.annotations.expanduser().resolve()
    annotations = load_jsonl(annotations_path)
    seen_ids: set[str | None] = set()
    duplicate_ids: set[str | None] = set()
    for annotation in annotations:
        task_id = annotation.get("id")
        if task_id in seen_ids:
            duplicate_ids.add(task_id)
        seen_ids.add(task_id)
    if duplicate_ids:
        labels = sorted(map(str, duplicate_ids))
        raise ValueError(f"duplicate annotation IDs: {', '.join(labels[:5])}")
    by_id = {annotation.get("id"): annotation for annotation in annotations}
    approved: list[tuple[dict, dict]] = []
    errors: list[str] = []
    for task_id, task in tasks.items():
        annotation = by_id.get(task_id)
        if not annotation or annotation.get("status") != "approved":
            if not args.allow_partial:
                errors.append(f"{task_id}: not approved")
            continue
        transcript = annotation.get("romanReference", "").strip()
        transcriber = annotation.get("transcriberId", "").strip()
        reviewer = annotation.get("reviewerId", "").strip()
        sample_errors: list[str] = []
        expected_silence = task.get("condition") == "silence"
        if expected_silence and transcript:
            sample_errors.append("silence reference must be empty")
        elif not expected_silence and not is_roman_reference(transcript):
            sample_errors.append("reference is empty or contains non-Latin letters/control characters")
        if (
            not PSEUDONYM.fullmatch(transcriber)
            or not PSEUDONYM.fullmatch(reviewer)
            or transcriber == reviewer
        ):
            sample_errors.append("requires distinct pseudonymous transcriber and reviewer IDs")
        if annotation.get("confidence") not in {"high", "medium", "low"}:
            sample_errors.append("confidence must be high, medium, or low")
        if sample_errors:
            errors.extend(f"{task_id}: {message}" for message in sample_errors)
        else:
            approved.append((task, annotation))
    unknown = sorted(set(by_id) - set(tasks))
    if unknown:
        errors.append(f"annotations contain unknown task IDs: {', '.join(unknown[:5])}")
    if errors:
        raise ValueError("corpus validation failed:\n- " + "\n- ".join(errors[:30]))
    if not approved:
        raise ValueError("no approved samples")

    output = args.out.expanduser().resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    kind = metadata["kind"]
    header = {
        "type": "corpus_header",
        "schemaVersion": 1,
        "dataset": f"my-assistant/{kind}",
        "createdAt": datetime.now(timezone.utc).isoformat(),
        "license": "private-evaluation-only",
        "referenceStyle": "roman",
        "referenceQuality": "human-double-reviewed",
        "samples": len(approved),
        "packId": metadata["packId"],
        "source": metadata,
        "annotationSha256": hash_file(annotations_path),
        "packMetadataSha256": hash_file(pack_dir / "pack.json"),
        "tasksSha256": hash_file(pack_dir / "tasks.json"),
        "privacy": "pseudonymous IDs; do not commit audio, annotations, transcripts, or raw reports",
    }
    with output.open("w", encoding="utf-8") as handle:
        handle.write(json.dumps(header, ensure_ascii=False) + "\n")
        for task, annotation in approved:
            record = {
                "id": task["id"],
                "audio": task["audio"],
                "ref_text": annotation["romanReference"].strip(),
                "language": "auto",
                "referenceStyle": "roman",
                "referenceQuality": "human-double-reviewed",
                "expectedSilence": task.get("condition") == "silence",
                "confidence": annotation["confidence"],
                "transcriberId": annotation["transcriberId"],
                "reviewerId": annotation["reviewerId"],
            }
            for key in (
                "sourceRow",
                "condition",
                "acousticOrigin",
                "speakerId",
                "consentId",
                "device",
                "locationType",
            ):
                if key in task:
                    record[key] = task[key]
            handle.write(json.dumps(record, ensure_ascii=False) + "\n")
    print(output)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    roman = subparsers.add_parser("create-roman-pack")
    roman.add_argument("--source-manifest", type=Path, required=True)
    roman.add_argument("--out-dir", type=Path, required=True)
    roman.add_argument("--count", type=int, default=500)
    roman.add_argument("--seed", type=int, default=42)
    roman.set_defaults(func=create_roman_pack)
    real = subparsers.add_parser("create-real-pack")
    real.add_argument("--captures", type=Path, required=True)
    real.add_argument("--out-dir", type=Path, required=True)
    real.set_defaults(func=create_real_pack)
    finish = subparsers.add_parser("finalize")
    finish.add_argument("--pack", type=Path, required=True)
    finish.add_argument("--annotations", type=Path, required=True)
    finish.add_argument("--out", type=Path, required=True)
    finish.add_argument("--allow-partial", action="store_true")
    finish.set_defaults(func=finalize)
    return parser


def main() -> int:
    args = build_parser().parse_args()
    if getattr(args, "count", 1) < 1:
        raise ValueError("--count must be positive")
    args.func(args)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=os.sys.stderr)
        raise SystemExit(1)
