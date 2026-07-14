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

    def test_multipart_includes_requested_translation_mode(self):
        import tempfile

        with tempfile.NamedTemporaryFile(suffix=".wav") as audio:
            content_type, body = BENCHMARK.multipart(Path(audio.name), "auto", True)
        self.assertIn("multipart/form-data", content_type)
        self.assertIn(b'name="language"\r\n\r\nauto', body)
        self.assertIn(b'name="translate"\r\n\r\ntrue', body)

    def test_character_f2_is_bounded_and_rewards_matching_text(self):
        self.assertEqual(BENCHMARK.character_f2([("hello world", "hello world")]), 1.0)
        self.assertEqual(BENCHMARK.character_f2([("abc", "xyz")]), 0.0)
        self.assertGreater(
            BENCHMARK.character_f2([("the satellite reflects it", "the satellite reflects it back")]),
            0.7,
        )

    def test_english_asr_is_comparable_for_multilingual_and_roman_models(self):
        self.assertTrue(BENCHMARK.styles_comparable("english", "mixed"))
        self.assertTrue(BENCHMARK.styles_comparable("english", "roman"))
        self.assertTrue(BENCHMARK.styles_comparable("english", "english"))

    def test_hinglish_orthographies_require_style_matched_references(self):
        self.assertFalse(BENCHMARK.styles_comparable("mixed", "roman"))
        self.assertFalse(BENCHMARK.styles_comparable("roman", "mixed"))
        self.assertTrue(BENCHMARK.styles_comparable("mixed", "mixed"))

    def test_integration_only_catalog_entry_resolves_to_a_safe_placeholder(self):
        model = {
            "id": "custom-runtime",
            "status": "integration-required",
            "source": {"repo": "example/model"},
        }
        path = BENCHMARK.resolve_model_path(model, Path("/tmp/bench"))
        self.assertEqual(path, Path("/tmp/bench/models/custom-runtime/UNAVAILABLE"))
        with self.assertRaisesRegex(ValueError, "integration-required"):
            BENCHMARK.verify_model(path, model)


if __name__ == "__main__":
    unittest.main()
