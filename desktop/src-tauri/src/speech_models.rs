use crate::speech_audio::SpeechPreprocessingPolicy;

pub const DEFAULT_MODEL_ID: &str = "whisper-large-v3-turbo";

const ENGLISH: &[&str] = &["en"];
const WHISPER_LANGUAGES: &[&str] = &[
    "en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it",
    "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur",
    "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn",
    "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si",
    "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo",
    "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln",
    "ha", "ba", "jw", "su", "yue",
];
const NEMOTRON_LANGUAGES: &[&str] = &[
    "en", "es", "fr", "it", "pt", "nl", "de", "tr", "ru", "ar", "hi", "ja", "ko", "vi", "uk", "pl",
    "sv", "cs", "nb", "da", "bg", "fi", "hr", "sk", "zh", "hu", "ro", "et",
];

#[derive(Clone, Copy, Debug)]
pub struct SpeechModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub family: &'static str,
    pub filename: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
    pub best_for: &'static str,
    pub description: &'static str,
    pub abilities: &'static [&'static str],
    pub badge: Option<&'static str>,
    pub license: &'static str,
    pub source_url: &'static str,
    pub supported_languages: &'static [&'static str],
    pub supports_language_detection: bool,
    pub supports_translation: bool,
    pub fixed_language: Option<&'static str>,
    pub output_style: &'static str,
    pub audio_profile: &'static str,
    pub preprocessing: SpeechPreprocessingPolicy,
}

// This is deliberately a small, useful library rather than every checkpoint on the Hub. Artifact URLs are
// revision-pinned and every download is size- and SHA-256-verified before it can be selected. The selection
// borrows Handy's model-library product pattern; metadata and decisions are maintained by My Assistant.
pub const SPEECH_MODELS: &[SpeechModelSpec] = &[
    SpeechModelSpec {
        id: "whisper-large-v3-turbo",
        name: "Whisper Large v3 Turbo",
        family: "Whisper",
        filename: "whisper-large-v3-turbo-Q8_0.gguf",
        url: "https://huggingface.co/handy-computer/whisper-large-v3-turbo-gguf/resolve/d222c9f621c1128299248f2ded4d8a1820519780/whisper-large-v3-turbo-Q8_0.gguf",
        sha256: "d5e65f2b0828802ae2c231673d31982cebe3a778c95d9494a9f3efee6bd17448",
        size: 886_381_824,
        best_for: "Everyday multilingual dictation",
        description: "The current balanced default: strong quality without Large v3's slower decoding.",
        abilities: &["100 languages", "Auto language detection", "Punctuation"],
        badge: Some("Recommended"),
        license: "Apache-2.0",
        source_url: "https://huggingface.co/openai/whisper-large-v3-turbo",
        supported_languages: WHISPER_LANGUAGES,
        supports_language_detection: true,
        supports_translation: false,
        fixed_language: None,
        output_style: "Original language",
        audio_profile: "Whisper noise guard",
        preprocessing: SpeechPreprocessingPolicy::VadCompact,
    },
    SpeechModelSpec {
        id: "parakeet-unified-en-0.6b",
        name: "Parakeet Unified EN 0.6B",
        family: "Parakeet",
        filename: "parakeet-unified-en-0.6b-Q8_0.gguf",
        url: "https://huggingface.co/handy-computer/parakeet-unified-en-0.6b-gguf/resolve/7e948f21b7bdbac698d3318db9d350f1096f3b6c/parakeet-unified-en-0.6b-Q8_0.gguf",
        sha256: "4b50b6dd862bf6e346929aaf4f5eaacec003bfa3f56462d6c874b41ef2f38795",
        size: 731_357_568,
        best_for: "Fast, accurate English",
        description: "English specialist with a streaming-ready architecture and token timestamps.",
        abilities: &["English", "Streaming-ready", "Token timestamps"],
        badge: Some("Best for English"),
        license: "CC-BY-4.0",
        source_url: "https://huggingface.co/nvidia/parakeet-unified-en-0.6b",
        supported_languages: ENGLISH,
        supports_language_detection: false,
        supports_translation: false,
        fixed_language: Some("en"),
        output_style: "English",
        audio_profile: "Streaming speech gate",
        preprocessing: SpeechPreprocessingPolicy::VadGate,
    },
    SpeechModelSpec {
        id: "nemotron-speech-streaming-en-0.6b",
        name: "Nemotron Speech Streaming EN",
        family: "Nemotron",
        filename: "nemotron-speech-streaming-en-0.6b-Q8_0.gguf",
        url: "https://huggingface.co/handy-computer/nemotron-speech-streaming-en-0.6b-gguf/resolve/7d9b719206789e4068d87c6398262ab4dfd4e45d/nemotron-speech-streaming-en-0.6b-Q8_0.gguf",
        sha256: "90d8c89714cd31efc88be62a40c6b2bea57e0cc2063af1ffe2c28f1a228ca110",
        size: 729_650_176,
        best_for: "Live English speech",
        description: "NVIDIA's English-only streaming model with punctuation and token timestamps.",
        abilities: &["English", "Streaming-ready", "Token timestamps"],
        badge: None,
        license: "OpenMDW-1.1",
        source_url: "https://huggingface.co/nvidia/nemotron-speech-streaming-en-0.6b",
        supported_languages: ENGLISH,
        supports_language_detection: false,
        supports_translation: false,
        fixed_language: Some("en"),
        output_style: "English",
        audio_profile: "Streaming speech gate",
        preprocessing: SpeechPreprocessingPolicy::VadGate,
    },
    SpeechModelSpec {
        id: "whisper-tiny-en",
        name: "Whisper Tiny (English)",
        family: "Whisper",
        filename: "whisper-tiny.en-Q8_0.gguf",
        url: "https://huggingface.co/handy-computer/whisper-tiny.en-gguf/resolve/becb8bcb804405dc97b380a523d9975888820986/whisper-tiny.en-Q8_0.gguf",
        sha256: "e8c9b73c06344307d8b346e07fbe93dd88d894627854bcff31523f1ce44394fa",
        size: 45_904_544,
        best_for: "Maximum speed on older Macs",
        description: "The smallest English option. Very fast, with the largest accuracy trade-off.",
        abilities: &["English", "Very small", "Fastest Whisper"],
        badge: None,
        license: "Apache-2.0",
        source_url: "https://huggingface.co/openai/whisper-tiny.en",
        supported_languages: ENGLISH,
        supports_language_detection: false,
        supports_translation: false,
        fixed_language: Some("en"),
        output_style: "English",
        audio_profile: "Whisper noise guard",
        preprocessing: SpeechPreprocessingPolicy::VadCompact,
    },
    SpeechModelSpec {
        id: "whisper-base-en",
        name: "Whisper Base (English)",
        family: "Whisper",
        filename: "whisper-base.en-Q8_0.gguf",
        url: "https://huggingface.co/handy-computer/whisper-base.en-gguf/resolve/cf0804db15fb341d00c9274b90da9cbb4fe2e5c6/whisper-base.en-Q8_0.gguf",
        sha256: "3b46ca40bccbf7609c68d88a36d96077a04ca7c87f2060ede06f129fac3e7652",
        size: 84_886_208,
        best_for: "Lightweight English dictation",
        description: "A small English model with a better quality-speed balance than Tiny.",
        abilities: &["English", "Small download", "Fast"],
        badge: None,
        license: "Apache-2.0",
        source_url: "https://huggingface.co/openai/whisper-base.en",
        supported_languages: ENGLISH,
        supports_language_detection: false,
        supports_translation: false,
        fixed_language: Some("en"),
        output_style: "English",
        audio_profile: "Whisper noise guard",
        preprocessing: SpeechPreprocessingPolicy::VadCompact,
    },
    SpeechModelSpec {
        id: "whisper-small-en",
        name: "Whisper Small (English)",
        family: "Whisper",
        filename: "whisper-small.en-Q8_0.gguf",
        url: "https://huggingface.co/handy-computer/whisper-small.en-gguf/resolve/41b0f75fd44415ba127a5356c5ba9ed450c1debd/whisper-small.en-Q8_0.gguf",
        sha256: "9614e6b7fda2d26018e4f268aece8ca25a83296ea0b534169a585b740bfd71ef",
        size: 269_674_144,
        best_for: "Fast English with better accuracy",
        description: "A practical middle ground for English on memory-constrained Macs.",
        abilities: &["English", "Fast", "Balanced size"],
        badge: None,
        license: "Apache-2.0",
        source_url: "https://huggingface.co/openai/whisper-small.en",
        supported_languages: ENGLISH,
        supports_language_detection: false,
        supports_translation: false,
        fixed_language: Some("en"),
        output_style: "English",
        audio_profile: "Whisper noise guard",
        preprocessing: SpeechPreprocessingPolicy::VadCompact,
    },
    SpeechModelSpec {
        id: "whisper-medium-en",
        name: "Whisper Medium (English)",
        family: "Whisper",
        filename: "whisper-medium.en-Q8_0.gguf",
        url: "https://huggingface.co/handy-computer/whisper-medium.en-gguf/resolve/f25c70d9095dcfdad187ebb3b113d157b414aee8/whisper-medium.en-Q8_0.gguf",
        sha256: "03d7257fef498750ce272631bc6a34de322fc2b438aab5c268ff49dfd1b64c49",
        size: 831_460_928,
        best_for: "High-quality English",
        description: "More accurate English at the cost of slower transcription and higher memory use.",
        abilities: &["English", "Higher accuracy", "Punctuation"],
        badge: None,
        license: "Apache-2.0",
        source_url: "https://huggingface.co/openai/whisper-medium.en",
        supported_languages: ENGLISH,
        supports_language_detection: false,
        supports_translation: false,
        fixed_language: Some("en"),
        output_style: "English",
        audio_profile: "Whisper noise guard",
        preprocessing: SpeechPreprocessingPolicy::VadCompact,
    },
    SpeechModelSpec {
        id: "whisper-medium-multilingual",
        name: "Whisper Medium (Multilingual)",
        family: "Whisper",
        filename: "whisper-medium-Q5_K_M.gguf",
        url: "https://huggingface.co/handy-computer/whisper-medium-gguf/resolve/ec78f06fded51aa82cde751678b78f76f78c8b7f/whisper-medium-Q5_K_M.gguf",
        sha256: "4e2a8904a866b3aa7ef70d7640ec6abc5f0a05524cd950ea4b66ace12122bf53",
        size: 582_746_048,
        best_for: "Translation to English",
        description: "The best translation option in the current local benchmark, with broad multilingual transcription.",
        abilities: &["99 languages", "Auto language detection", "Translate to English"],
        badge: Some("Best for translation"),
        license: "Apache-2.0",
        source_url: "https://huggingface.co/openai/whisper-medium",
        supported_languages: WHISPER_LANGUAGES,
        supports_language_detection: true,
        supports_translation: true,
        fixed_language: None,
        output_style: "Original language or English translation",
        audio_profile: "Whisper noise guard",
        preprocessing: SpeechPreprocessingPolicy::VadCompact,
    },
    SpeechModelSpec {
        id: "whisper-large-v3",
        name: "Whisper Large v3",
        family: "Whisper",
        filename: "whisper-large-v3-Q5_K_M.gguf",
        url: "https://huggingface.co/handy-computer/whisper-large-v3-gguf/resolve/e3e29bee6389c7da4a141406f07bb80ddac5337c/whisper-large-v3-Q5_K_M.gguf",
        sha256: "6053d0fd69a0fd48b8fea5ea7a52b9e0cde389343566fa30e453a1b2b258dc38",
        size: 1_161_143_008,
        best_for: "Maximum Whisper quality",
        description: "The slower, heavier Whisper option for users who prefer quality over latency.",
        abilities: &["100 languages", "Auto language detection", "Highest Whisper quality"],
        badge: None,
        license: "Apache-2.0",
        source_url: "https://huggingface.co/openai/whisper-large-v3",
        supported_languages: WHISPER_LANGUAGES,
        supports_language_detection: true,
        supports_translation: true,
        fixed_language: None,
        output_style: "Original language or English translation",
        audio_profile: "Whisper noise guard",
        preprocessing: SpeechPreprocessingPolicy::VadCompact,
    },
    SpeechModelSpec {
        id: "nemotron-3.5-asr-streaming-0.6b",
        name: "Nemotron Streaming 3.5",
        family: "Nemotron",
        filename: "nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf",
        url: "https://huggingface.co/handy-computer/nemotron-3.5-asr-streaming-0.6b-gguf/resolve/6d44e540bc31b0de1dbe174a3cea87f53a7f22fb/nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf",
        sha256: "b94545b313b3223fda7b2857a52681da813935c2127643d1e9ff0c23d988089c",
        size: 751_094_240,
        best_for: "Fast multilingual speech",
        description: "Streaming-ready NVIDIA model covering 28 languages in this local build.",
        abilities: &["28 languages", "Auto language detection", "Streaming-ready"],
        badge: Some("Multilingual"),
        license: "OpenMDW-1.1",
        source_url: "https://huggingface.co/nvidia/nemotron-3.5-asr-streaming-0.6b",
        supported_languages: NEMOTRON_LANGUAGES,
        supports_language_detection: true,
        supports_translation: false,
        fixed_language: None,
        output_style: "Original language",
        audio_profile: "Streaming speech gate",
        preprocessing: SpeechPreprocessingPolicy::VadGate,
    },
    SpeechModelSpec {
        id: "oriserve-hinglish-apex",
        name: "Oriserve Hinglish Apex",
        family: "Whisper",
        filename: "ggml-apex-hinglish-q8_0.bin",
        url: "https://huggingface.co/Marquestra/Whisper-Hindi2Hinglish-Apex-GGML/resolve/d1de3ff618856e5675c47d3158ca820506fb4d9e/ggml-apex-hinglish-q8_0.bin",
        sha256: "bef6d9d571e9e7bd3cd2ebba8b2d5c9cc4731e210d6750ae6a9c621a87e60d4c",
        size: 874_188_075,
        best_for: "Roman Hinglish",
        description: "Fast Large-v3-Turbo fine-tune that writes Hindi-English speech in Roman script.",
        abilities: &["Roman Hinglish", "Indian accents", "Noise-focused training"],
        badge: Some("Best for Hinglish"),
        license: "Apache-2.0",
        source_url: "https://huggingface.co/Oriserve/Whisper-Hindi2Hinglish-Apex",
        supported_languages: ENGLISH,
        supports_language_detection: false,
        supports_translation: false,
        fixed_language: Some("en"),
        output_style: "Roman Hinglish",
        audio_profile: "Hinglish fine-tune noise gate",
        preprocessing: SpeechPreprocessingPolicy::VadGate,
    },
    SpeechModelSpec {
        id: "oriserve-hinglish-prime",
        name: "Oriserve Hinglish Prime",
        family: "Whisper",
        filename: "ggml-hindi2hinglish-prime.bin",
        url: "https://huggingface.co/curiophile/whisper-hindi2hinglish-ggml/resolve/ea0d52c8fe269d87ae03e0f5a4cbd0b5673b35e2/ggml-hindi2hinglish-prime.bin",
        sha256: "60bb800a35c1df7400020ed2e13981027b2c6caff71a318f321f6108838f0bad",
        size: 1_177_039_883,
        best_for: "Roman Hinglish quality",
        description: "Large-v3 Hinglish fine-tune for users willing to trade speed for a heavier model.",
        abilities: &["Roman Hinglish", "Indian accents", "Noise-focused training"],
        badge: None,
        license: "Apache-2.0",
        source_url: "https://huggingface.co/Oriserve/Whisper-Hindi2Hinglish-Prime",
        supported_languages: ENGLISH,
        supports_language_detection: false,
        supports_translation: false,
        fixed_language: Some("en"),
        output_style: "Roman Hinglish",
        audio_profile: "Hinglish fine-tune noise gate",
        preprocessing: SpeechPreprocessingPolicy::VadGate,
    },
];

pub fn speech_model(id: &str) -> Option<&'static SpeechModelSpec> {
    SPEECH_MODELS.iter().find(|model| model.id == id)
}

pub fn default_speech_model() -> &'static SpeechModelSpec {
    speech_model(DEFAULT_MODEL_ID).expect("default speech model is present in the built-in catalog")
}

/// True only for artifacts whose loaded architecture exposes the runtime's genuine incremental
/// `stream_begin/feed/finalize` contract. Whisper checkpoints are deliberately excluded: repeatedly
/// decoding a growing Whisper buffer looks live, but it is slower, unstable, and not model streaming.
pub fn supports_live_streaming(model: &SpeechModelSpec) -> bool {
    matches!(
        model.id,
        "parakeet-unified-en-0.6b"
            | "nemotron-speech-streaming-en-0.6b"
            | "nemotron-3.5-asr-streaming-0.6b"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn catalog_has_unique_safe_ids_and_pinned_artifacts() {
        let mut ids = HashSet::new();
        for model in SPEECH_MODELS {
            assert!(ids.insert(model.id), "duplicate model id {}", model.id);
            assert!(model
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.'));
            assert!(!model.url.contains("/resolve/main/"));
            assert_eq!(model.sha256.len(), 64);
            assert!(model.sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(model.size > 1_000_000);
        }
        assert_eq!(default_speech_model().id, DEFAULT_MODEL_ID);
    }

    #[test]
    fn requested_specialists_are_kept() {
        for id in [
            "parakeet-unified-en-0.6b",
            "nemotron-speech-streaming-en-0.6b",
            "nemotron-3.5-asr-streaming-0.6b",
            "oriserve-hinglish-apex",
            "oriserve-hinglish-prime",
        ] {
            assert!(speech_model(id).is_some(), "missing requested model {id}");
        }
    }

    #[test]
    fn live_streaming_is_exposed_only_for_native_streaming_architectures() {
        let enabled: Vec<&str> = SPEECH_MODELS
            .iter()
            .filter(|model| supports_live_streaming(model))
            .map(|model| model.id)
            .collect();
        assert_eq!(
            enabled,
            vec![
                "parakeet-unified-en-0.6b",
                "nemotron-speech-streaming-en-0.6b",
                "nemotron-3.5-asr-streaming-0.6b"
            ]
        );
        assert!(!supports_live_streaming(default_speech_model()));
    }
}
