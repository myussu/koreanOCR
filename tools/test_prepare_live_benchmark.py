"""Integrity checks for prepared subtitle data; no model or video decoder is started."""
import json
import shutil
import unittest
import uuid
from contextlib import contextmanager
from pathlib import Path

from prepare_live_benchmark import checked_path, digest, prepare, restore_context, reviewed_timing, validate_cases, write_json, write_lines


class CorpusIntegrity(unittest.TestCase):
    @contextmanager
    def temporary_folder(self):
        scratch_root = (Path(__file__).resolve().parents[1] / "target/live-benchmark-test-temp").resolve()
        folder = scratch_root / str(uuid.uuid4())
        folder.mkdir(parents=True)
        try:
            yield folder
        finally:
            resolved = folder.resolve()
            if not resolved.is_relative_to(scratch_root) or resolved == scratch_root:
                raise ValueError("Refusing to clean outside the test scratch directory")
            shutil.rmtree(resolved)

    def test_duplicate_ids_cannot_be_used_to_inflate_a_comparison(self):
        case = dict(id="same", scene="scene", source="Korean", reference="Meaning", origin="fixture")
        with self.assertRaisesRegex(ValueError, "Duplicate"):
            validate_cases([case, case])

    def test_case_ids_cannot_turn_evidence_destinations_into_paths(self):
        case = dict(id="../outside", scene="scene", source="source", reference="meaning", origin="fixture")
        with self.assertRaisesRegex(ValueError, "Case IDs"):
            validate_cases([case])

    def test_missing_dialogue_is_inserted_in_time_order_without_changing_the_legacy_set(self):
        legacy = [dict(id=str(n), scene="elden_ring:scene", source="source", reference="meaning", origin="fixture")
                  for n in (1, 3)]
        additions = [dict(id="2", scene="elden_ring:scene", source="source", reference="meaning", origin="fixture",
                          game="elden_ring", evidence_seconds=2)]
        items = {"1": {"start_seconds": 1}, "3": {"start_seconds": 3}}
        self.assertEqual([row["id"] for row in restore_context(legacy, additions, items)], ["1", "2", "3"])
        self.assertEqual([row["id"] for row in legacy], ["1", "3"])

    def test_a_scene_cannot_reenter_after_another_scene_has_reset_history(self):
        cases = [dict(id=str(n), scene=scene, source="source", reference="reference", origin="fixture")
                 for n, scene in enumerate(("one", "two", "one"))]
        with self.assertRaisesRegex(ValueError, "interleaved"):
            validate_cases(cases)

    def test_automatic_boundary_proposals_are_not_promoted_as_visually_reviewed(self):
        self.assertIsNone(reviewed_timing(dict(onset=[1, 2], offset=[3, 4], review_status="automatic")))

    def test_a_timing_without_a_visible_interior_is_refused(self):
        with self.assertRaisesRegex(ValueError, "visible interior"):
            reviewed_timing(dict(onset=[1, 3], offset=[2, 4], review_status="agent_visually_checked_brackets"))

    def test_nonfinite_timestamps_are_refused(self):
        with self.assertRaises(ValueError):
            reviewed_timing(dict(onset=[1, float("nan")], offset=[3, 4]))

    def test_evidence_cannot_escape_the_declared_source_directory(self):
        with self.temporary_folder() as folder:
            root = Path(folder) / "source"
            root.mkdir()
            (Path(folder) / "outside.png").write_bytes(b"outside")
            with self.assertRaisesRegex(ValueError, "inside"):
                checked_path(root, "../outside.png")

    def test_localization_and_unverified_transcripts_never_become_scoring_references(self):
        with self.temporary_folder() as folder:
            root = Path(folder)
            source = root / "source"
            (source / "transcripts").mkdir(parents=True)
            (source / "video.mp4").write_bytes(b"not decoded by this test")
            (source / "evidence.png").write_bytes(b"preserved evidence bytes")
            write_json(source / "transcripts/elden_ring_verified_dataset.json", dict(video_path="video.mp4", items=[
                dict(id="er", evidence_screenshot="evidence.png", english_reference="different localization")]))
            write_json(source / "transcripts/unverified_dialogue.json", dict(game="unverified", scenes=[
                dict(scene="scene", dialogues=[dict(ko="unverified Korean", en="untrusted English")])]))
            legacy = root / "legacy.jsonl"
            write_lines(legacy, [dict(id="er", scene="er", source="빛바랜 자여\n…", reference="Meaning aid", origin="fixture")])
            before = digest(legacy)
            (root / "new.png").write_bytes(b"new evidence")
            additions = root / "additions.json"
            write_json(additions, dict(cases=[dict(id="new", scene="sekiro:scene", source="source", reference="reference",
                       origin="fixture", game="sekiro", source_status="agent_visually_transcribed",
                       evidence="new.png", video_relative_path="video.mp4")]))
            timings = root / "timings.json"
            write_json(timings, dict(er=dict(onset=[1, 2], offset=[3, 4], review_status="agent_visually_checked_brackets")))
            out = root / "pack"
            manifest = prepare(source, legacy, additions, timings, out)
            rows = json.loads((out / "annotations/cases.json").read_text(encoding="utf-8"))
            self.assertEqual(rows[0]["reference"], "Meaning aid")
            self.assertEqual(rows[0]["official_english_localization"], "different localization")
            self.assertEqual(rows[0]["source"], "빛바랜 자여\n…")
            self.assertEqual(manifest["pending_transcript_rows"], 1)
            self.assertEqual(manifest["certified_human_references"], 0)
            self.assertEqual(manifest["complete_scene_recall_eligible"], 0)
            self.assertEqual(digest(legacy), before)
            self.assertEqual(digest(out / "corpora/legacy-70.jsonl"), before)
            self.assertTrue(all(digest(out / name) == value for name, value in manifest["outputs"].items()))
            for corpus in (out / "corpora").glob("*.jsonl"):
                self.assertNotIn("unverified Korean", corpus.read_text(encoding="utf-8"))
                for line in corpus.read_text(encoding="utf-8").splitlines():
                    self.assertIsInstance(json.loads(line)["tags"], list)
            with self.assertRaisesRegex(ValueError, "immutable"):
                prepare(source, legacy, additions, timings, out)


if __name__ == "__main__":
    unittest.main()
