import importlib.util
import json
import sys
import tempfile
import unittest
import wave
from argparse import Namespace
from pathlib import Path


HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))


def load_module(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, HERE / filename)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader
    spec.loader.exec_module(module)
    return module


CORPUS = load_module("speech_corpus", "corpus.py")
MATRIX = load_module("speech_matrix", "run_matrix.py")
CAPTURE = load_module("speech_capture", "capture_real_noise.py")


class CorpusTests(unittest.TestCase):
    def test_roman_validator_rejects_devanagari(self):
        self.assertTrue(CORPUS.is_roman_reference("Kal client meeting hai at 4 pm."))
        self.assertTrue(CORPUS.is_roman_reference("Mujhe café mein milo."))
        self.assertFalse(CORPUS.is_roman_reference("कल client meeting है"))
        self.assertFalse(CORPUS.is_roman_reference("kalा meeting hai"))
        self.assertFalse(CORPUS.is_roman_reference(""))

    def test_versioned_field_prompt_resolves_without_becoming_a_reference(self):
        args = Namespace(prompt_id="hf-001", prompt_text="")
        CAPTURE.resolve_prompt(args)
        self.assertIn("action items", args.prompt_text)

    def test_pack_is_blind_and_final_reference_is_double_reviewed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            clips = root / "clips"
            clips.mkdir()
            audio = clips / "one.wav"
            with wave.open(str(audio), "wb") as output:
                output.setnchannels(1)
                output.setsampwidth(2)
                output.setframerate(16_000)
                output.writeframes(b"\0\0" * 160)
            source = root / "source.jsonl"
            source.write_text(
                json.dumps(
                    {
                        "type": "corpus_header",
                        "dataset": "example/source",
                        "revision": "abc",
                        "license": "evaluation-only",
                    }
                )
                + "\n"
                + json.dumps(
                    {
                        "id": "one",
                        "audio": "clips/one.wav",
                        "ref_text": "यह hidden mixed reference है",
                        "sourceRow": 7,
                    },
                    ensure_ascii=False,
                )
                + "\n"
            )
            pack = root / "pack"
            CORPUS.create_roman_pack(
                Namespace(source_manifest=source, out_dir=pack, count=1, seed=42)
            )
            tasks_text = (pack / "tasks.json").read_text()
            self.assertNotIn("hidden mixed reference", tasks_text)
            annotations = root / "annotations.jsonl"
            annotations.write_text(
                json.dumps(
                    {
                        "id": "one",
                        "romanReference": "Yeh human Roman reference hai.",
                        "transcriberId": "annotator-a",
                        "reviewerId": "annotator-b",
                        "status": "approved",
                        "confidence": "high",
                        "notes": "",
                    }
                )
                + "\n"
            )
            output = root / "roman.jsonl"
            CORPUS.finalize(
                Namespace(pack=pack, annotations=annotations, out=output, allow_partial=False)
            )
            records = CORPUS.load_jsonl(output)
            self.assertEqual(records[0]["referenceQuality"], "human-double-reviewed")
            self.assertEqual(len(records[0]["annotationSha256"]), 64)
            self.assertEqual(records[1]["ref_text"], "Yeh human Roman reference hai.")

            annotations.write_text(annotations.read_text() * 2)
            with self.assertRaisesRegex(ValueError, "duplicate annotation IDs"):
                CORPUS.finalize(
                    Namespace(pack=pack, annotations=annotations, out=output, allow_partial=False)
                )


class MatrixTests(unittest.TestCase):
    def test_catalog_policy_is_explicit(self):
        self.assertEqual(
            MATRIX.preprocessing_policy(
                {"id": "qwen", "preprocessing": {"policy": "vad-gate"}}
            ),
            "vad-gate",
        )
        with self.assertRaisesRegex(ValueError, "no supported"):
            MATRIX.preprocessing_policy({"id": "missing"})

    def test_model_receipt_contains_provenance_without_a_local_path(self):
        receipt = MATRIX.model_receipt(
            {
                "id": "candidate",
                "license": "Apache-2.0",
                "source": {
                    "repo": "owner/model",
                    "revision": "abc",
                    "file": "model.safetensors",
                    "sha256": "source-hash",
                },
                "artifact": {
                    "file": "model.gguf",
                    "size": 123,
                    "sha256": "artifact-hash",
                    "quant": "Q6_K",
                },
            }
        )
        self.assertEqual(receipt["runtimeArtifact"]["sha256"], "artifact-hash")
        self.assertNotIn("localPaths", json.dumps(receipt))

    def test_summary_reports_cross_trial_hypothesis_instability(self):
        base = {
            "id": "candidate",
            "label": "Candidate",
            "referenceStyle": "roman",
            "referenceQuality": "human-double-reviewed",
            "scoreComparable": True,
            "samples": 1,
            "scriptSafeWer": 0.1,
            "characterF2": 0.9,
            "rtf": 0.2,
            "peakRssMiB": 500.0,
            "truncatedSamples": 0,
            "degenerateSamples": 0,
        }
        first = {**base, "results": [{"id": "one", "hypText": "kal meeting hai"}]}
        second = {**base, "results": [{"id": "one", "hypText": "kal ek meeting hai"}]}
        summary = MATRIX.summarize_model(
            "candidate",
            [
                {"models": [first], "_path": "/tmp/one.json"},
                {"models": [second], "_path": "/tmp/two.json"},
            ],
            "vad-gate",
        )
        self.assertEqual(summary["hypothesisDisagreementSamples"], 1)
        self.assertEqual(summary["trials"], 2)

    def test_trial_log_is_preserved_before_the_cache_log_is_reused(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "current.log"
            destination = root / "trial-01.log"
            source.write_text("repeat loop stopped\n")
            MATRIX.preserve_trial_log(
                {"models": [{"logPath": str(source)}]}, destination
            )
            self.assertEqual(destination.read_text(), source.read_text())


if __name__ == "__main__":
    unittest.main()
