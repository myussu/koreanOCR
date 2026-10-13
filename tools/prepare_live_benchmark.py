"""Freeze local subtitle material into separate benchmark suites without running models."""
import argparse
import csv
import hashlib
import json
import re
import shutil
from collections import Counter
from pathlib import Path


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def read_lines(path):
    return [json.loads(line) for line in path.read_text(encoding="utf-8-sig").splitlines() if line.strip()]


def validate_cases(cases):
    ids = set()
    completed_scenes = set()
    previous = None
    for case in cases:
        for key in ("id", "scene", "source", "reference", "origin"):
            if not isinstance(case.get(key), str) or not case[key].strip():
                raise ValueError(f"Empty or invalid {key}")
        if not re.fullmatch(r"[A-Za-z0-9_-]+", case["id"]):
            raise ValueError("Case IDs must use letters, digits, underscores or hyphens")
        if not isinstance(case.get("tags", []), list) or any(not isinstance(tag, str) for tag in case.get("tags", [])):
            raise ValueError("Tags must be a list of strings")
        if case["id"] in ids:
            raise ValueError(f"Duplicate case ID: {case['id']}")
        ids.add(case["id"])
        if case["scene"] != previous:
            if case["scene"] in completed_scenes:
                raise ValueError("A scene is interleaved with another scene; history would be incorrect")
            if previous is not None:
                completed_scenes.add(previous)
            previous = case["scene"]


def checked_path(root, relative):
    path = (root / relative).resolve()
    if not path.is_relative_to(root.resolve()):
        raise ValueError("Evidence must stay inside its source directory")
    if not path.is_file():
        raise ValueError(f"Missing evidence: {path}")
    return path


def reviewed_timing(value):
    if not value:
        return None
    for side in ("onset", "offset"):
        interval = value.get(side)
        if not isinstance(interval, list) or len(interval) != 2:
            raise ValueError("A timing bracket requires two endpoints")
        if any(not isinstance(n, (int, float)) or not 0 <= n < float("inf") for n in interval):
            raise ValueError("Timing endpoints must be finite and nonnegative")
        if interval[0] >= interval[1]:
            raise ValueError("Timing brackets must increase")
    if value["onset"][1] >= value["offset"][0]:
        raise ValueError("The subtitle must have a known visible interior")
    if value.get("review_status") != "agent_visually_checked_brackets":
        return None
    return value


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def write_lines(path, cases):
    validate_cases(cases)
    # The Rust comparison reader requires tags even when a case has no labels.
    path.write_text("".join(json.dumps({"tags": [], **case}, ensure_ascii=False) + "\n" for case in cases), encoding="utf-8")


def restore_context(legacy, additions, original_items, scenes=None):
    if scenes is None:
        scenes = list(dict.fromkeys(case["scene"] for case in additions if case["game"] == "elden_ring"))
    if len(scenes) != len(set(scenes)):
        raise ValueError("Restored scene names must be unique")
    result = []
    for scene in scenes:
        ordered = []
        for case in legacy + additions:
            if case["scene"] != scene:
                continue
            when = case.get("evidence_seconds", original_items.get(case["id"], {}).get("start_seconds"))
            if not isinstance(when, (int, float)) or not 0 <= when < float("inf"):
                raise ValueError("Restored dialogue requires a finite chronological position")
            ordered.append((when, case))
        result.extend(case for _, case in sorted(ordered, key=lambda pair: pair[0]))
    validate_cases(result)
    return result


def prepare(source_root, legacy_path, additions_path, timing_path, out):
    # Refuse replacement before copying anything. Source material is always read-only.
    if out.exists():
        raise ValueError("Use a new output folder; existing benchmark packs are immutable")
    legacy = read_lines(legacy_path)
    validate_cases(legacy)
    if any("tags" not in case for case in legacy):
        raise ValueError("The preserved legacy corpus must include tags for the existing CLI reader")
    addition_document = read_json(additions_path)
    additions = addition_document["cases"]
    for case in additions:
        case.setdefault("tags", [])
    validate_cases(additions)
    semantic_path = Path(__file__).parent / "testdata/live-semantic-checks.json"
    semantic = read_json(semantic_path)["cases"]
    for case in semantic:
        case["origin"] = "agent_authored_semantic_check"
        case["reference_status"] = "agent_authored_review_aid_not_certified_gold"
    validate_cases(semantic)
    all_ids = [case["id"] for case in legacy + additions + semantic]
    if len(all_ids) != len(set(all_ids)):
        raise ValueError("Case IDs overlap across suites")
    timings = read_json(timing_path)
    for timing in timings.values():
        if reviewed_timing(timing) is None:
            raise ValueError("An unreviewed timing cannot be promoted")
    unknown = set(timings) - set(all_ids)
    if unknown:
        raise ValueError(f"Timing IDs have no case: {sorted(unknown)}")
    dataset_path = source_root / "transcripts/elden_ring_verified_dataset.json"
    dataset = read_json(dataset_path)
    original_items = {item["id"]: item for item in dataset["items"]}
    pending = []
    for path in sorted((source_root / "transcripts").glob("*_dialogue.json")):
        raw = read_json(path)
        for scene_index, scene in enumerate(raw["scenes"]):
            for index, line in enumerate(scene["dialogues"]):
                pending.append(dict(id=f"pending:{path.stem}:{scene_index}:{index}", game=raw["game"],
                                    scene=scene["scene"], source=line["ko"], supplied_english=line.get("en"),
                                    supplied_speaker=line.get("character"), source_file=str(path.resolve()),
                                    status="unverified_source_and_alignment", eligible_for_quality_scoring=False,
                                    eligible_for_replay_scoring=False))
    evidence_sources = {}
    for case in additions:
        if case.get("source_status") != "agent_visually_transcribed":
            raise ValueError("New game lines require explicit visual transcription status")
        evidence_sources[case["id"]] = checked_path(additions_path.parent, case["evidence"])
    boundary_sources = {case_id: checked_path(timing_path.parent, timing["evidence_relative_path"])
                        for case_id, timing in timings.items() if timing.get("evidence_relative_path")}
    out.mkdir(parents=True)
    for name in ("sources", "corpora", "annotations", "evidence", "review", "replay"):
        (out / name).mkdir()
    sources = []
    for path in sorted((source_root / "transcripts").glob("*.json")) + [legacy_path, additions_path, timing_path, semantic_path]:
        destination = out / "sources" / path.name
        if destination.exists():
            raise ValueError("Source snapshot filenames collide")
        shutil.copyfile(path, destination)
        sources.append(dict(path=str(path.resolve()), snapshot=destination.relative_to(out).as_posix(),
                            sha256=digest(path), bytes=path.stat().st_size))
    shutil.copyfile(legacy_path, out / "corpora/legacy-70.jsonl")
    write_lines(out / "corpora/semantic-32.jsonl", semantic)
    new_game = [case for case in additions if case["game"] == "sekiro"]
    development = [case for case in additions if case["game"] != "sekiro"]
    write_lines(out / "corpora/new-game-validation.jsonl", new_game)
    write_lines(out / "corpora/added-dialogue.jsonl", development)
    context = restore_context(legacy, additions, original_items, addition_document.get("context_scenes"))
    write_lines(out / "corpora/restored-context.jsonl", context)
    annotations = []
    for case in legacy + additions + semantic:
        row = dict(case)
        row["reference_status"] = case.get("reference_status", "agent_meaning_aid_not_certified_gold")
        row["timing"] = reviewed_timing(timings.get(case["id"]))
        row["full_scene_coverage_verified"] = False
        if case["id"] in original_items:
            item = original_items[case["id"]]
            evidence = checked_path(source_root, item["evidence_screenshot"])
            row["original_annotation"] = item
            row["official_english_localization"] = item.get("english_reference")
            row["source_status"] = "prior_agent_midpoint_visual_check"
            row["video"] = str(checked_path(source_root, dataset["video_path"]))
        elif case["id"] in evidence_sources:
            evidence = evidence_sources[case["id"]]
            row["video"] = str(checked_path(source_root, case["video_relative_path"]))
        else:
            evidence = None
            row["source_status"] = case.get("source_status", "legacy_user_transcript_or_authored_check")
        if evidence:
            destination = out / "evidence" / f"{case['id']}.png"
            shutil.copyfile(evidence, destination)
            row["evidence"] = destination.relative_to(out).as_posix()
            row["evidence_sha256"] = digest(destination)
        if case["id"] in boundary_sources:
            destination = out / "evidence" / f"{case['id']}-boundaries.png"
            shutil.copyfile(boundary_sources[case["id"]], destination)
            row["timing"] = dict(row["timing"], evidence=destination.relative_to(out).as_posix(),
                                 evidence_sha256=digest(destination))
        annotations.append(row)
    write_json(out / "annotations/cases.json", annotations)
    (out / "review/pending.jsonl").write_text("".join(json.dumps(row, ensure_ascii=False) + "\n" for row in pending), encoding="utf-8")
    with (out / "review/semantic-review.tsv").open("w", encoding="utf-8", newline="") as stream:
        writer = csv.writer(stream, delimiter="\t")
        writer.writerow(["id", "korean", "meaning_aid", "source_status", "must_preserve", "reviewer", "reference_approved", "notes"])
        for row in annotations:
            writer.writerow([row["id"], row["source"], row["reference"], row["source_status"],
                             " | ".join(row.get("must_preserve", [])), "", "", ""])
    replay = [dict(id=row["id"], video=row["video"], source=row["source"], timing=row["timing"],
                   evidence=row["evidence"], crop=[300, 900, 1320, 150],
                   scope="selected_subtitle_presence_only", full_scene_coverage_verified=False)
              for row in annotations if row["timing"] and row.get("video")]
    write_json(out / "replay/selected-subtitles.json", replay)
    write_json(out / "review/optional-name-glossary.json", dict(
        status="Optional domain knowledge, not dialogue history; evaluate assisted and unassisted runs separately",
        entries=addition_document.get("optional_name_glossary", [])))
    write_json(out / "splits.json", dict(development=["legacy-70", "semantic-32", "added-dialogue", "restored-context"],
                                       validation=["new-game-validation"], split_unit="whole_scene",
                                       policy="Keep validation out of prompt selection; after using its results, call it regression material.",
                                       game_distribution=dict(Counter(row.get("game", row["origin"]) for row in annotations))))
    manifest = dict(schema_version=1, sources=sources,
                    videos=[dict(path=path, bytes=Path(path).stat().st_size, sha256=digest(Path(path)))
                            for path in sorted({row["video"] for row in annotations if row.get("video")})],
                    unique_cases=len(all_ids),
                    overlapping_suites=["restored-context"],
                    suites={"legacy-70": len(legacy), "semantic-32": len(semantic), "restored-context": len(context),
                            "new-game-validation": len(new_game), "added-dialogue": len(development)},
                    pending_transcript_rows=len(pending), timing_brackets=len(replay),
                    certified_human_references=0, complete_scene_recall_eligible=0,
                    limitations=["No model was run; new validation scenes have not been used to select prompts.",
                                 "Agent-created English references require independent semantic review.",
                                 "Timing brackets are conservative sample ranges, not frame-exact timestamps.",
                                 "Noisy OCR must be measured from evidence separately from clean transcription.",
                                 "Pending transcripts are never included in runnable comparison suites."],
                    outputs={p.relative_to(out).as_posix(): digest(p) for p in sorted(out.rglob("*")) if p.is_file()})
    write_json(out / "manifest.json", manifest)
    return manifest


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--legacy", type=Path, required=True)
    parser.add_argument("--additions", type=Path, required=True)
    parser.add_argument("--timings", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = prepare(args.source_root, args.legacy, args.additions, args.timings, args.out)
    except (ValueError, OSError, KeyError) as error:
        parser.exit(1, f"Cannot prepare benchmark: {error}\n")
    print(json.dumps({key: result[key] for key in ("suites", "pending_transcript_rows", "timing_brackets")}, indent=2))
