import importlib.util
import json
import tempfile
import unittest
from argparse import Namespace
from pathlib import Path


HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("acceptance_runner", HERE / "run.py")
RUNNER = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
SPEC.loader.exec_module(RUNNER)


class AcceptanceRunnerTests(unittest.TestCase):
    def test_case_ids_are_unique_and_platform_matrix_is_explicit(self):
        suites = RUNNER.cases()
        for suite in suites.values():
            ids = [case["id"] for case in suite["cases"]]
            self.assertEqual(len(ids), len(set(ids)))
        self.assertIn("google-meet-chrome", suites["two-person-meeting"]["platforms"])
        self.assertIn("zoom-native", suites["two-person-meeting"]["platforms"])

    def test_pass_requires_evidence(self):
        with tempfile.TemporaryDirectory() as temporary:
            session_path = Path(temporary) / "session.json"
            cases = RUNNER.cases()["right-command"]["cases"]
            session_path.write_text(
                json.dumps(
                    {
                        "schemaVersion": 1,
                        "sessionId": "test",
                        "suite": "right-command",
                        "results": {
                            case["id"]: {
                                "result": "pending",
                                "evidence": [],
                                "notes": "",
                                "recordedAt": None,
                            }
                            for case in cases
                        },
                    }
                )
            )
            with self.assertRaisesRegex(ValueError, "requires at least one"):
                RUNNER.record(
                    Namespace(
                        session=session_path,
                        case="RC-001",
                        result="pass",
                        evidence=[],
                        notes="",
                    )
                )

    def test_evidence_metadata_pins_file_content(self):
        with tempfile.TemporaryDirectory() as temporary:
            evidence = Path(temporary) / "observation.txt"
            evidence.write_text("physical Right Command edge observed\n")
            metadata = RUNNER.evidence_metadata([str(evidence)])
            self.assertEqual(metadata[0]["bytes"], evidence.stat().st_size)
            self.assertEqual(len(metadata[0]["sha256"]), 64)
            session = {
                "results": {
                    "RC-001": {
                        "result": "pass",
                        "evidence": [str(evidence)],
                        "evidenceMetadata": metadata,
                    }
                }
            }
            self.assertEqual(RUNNER.changed_pass_evidence(session), [])
            evidence.write_text("changed\n")
            self.assertEqual(RUNNER.changed_pass_evidence(session), ["RC-001"])


if __name__ == "__main__":
    unittest.main()
