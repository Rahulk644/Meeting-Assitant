import importlib.util
import unittest
from pathlib import Path


SPEC = importlib.util.spec_from_file_location("speech_benchmark", Path(__file__).with_name("run.py"))
BENCHMARK = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
SPEC.loader.exec_module(BENCHMARK)


class ScriptSafeScoringTests(unittest.TestCase):
    def test_preserves_devanagari_marks_and_lowercases_latin(self):
        self.assertEqual(
            BENCHMARK.script_safe_tokens("ठीक है, API's READY!"),
            ["ठीक", "है", "apis", "ready"],
        )

    def test_edit_counts(self):
        self.assertEqual(BENCHMARK.edit_counts(["a", "b", "c"], ["a", "x", "c"]), (1, 0, 0))
        self.assertEqual(BENCHMARK.edit_counts(["a", "b"], ["a"]), (0, 1, 0))
        self.assertEqual(BENCHMARK.edit_counts(["a"], ["a", "b"]), (0, 0, 1))

    def test_degenerate_output_detection(self):
        self.assertEqual(BENCHMARK.degenerate_reason("nan."), "non-finite-token")
        loop = " ".join(["he is the one who"] * 12)
        self.assertEqual(BENCHMARK.degenerate_reason(loop), "repetition-loop")
        self.assertIsNone(BENCHMARK.degenerate_reason("यह एक normal Hinglish sentence है"))


if __name__ == "__main__":
    unittest.main()
