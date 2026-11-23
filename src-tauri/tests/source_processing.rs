use clovy_lib::audio::turns::{
    coalesce_turns_for_transcription, detect_turns, normalize_wav_for_transcription,
    write_turn_wav, AudioTurn, DetectionSource,
};
use clovy_lib::audio::validation::{
    source_audio_passes_validation, validate_audio_artifact, AudioValidationConfig,
};
use clovy_lib::domain::processing::{
    build_transcription_context, build_transcription_dictionary, coalesce_source_transcripts,
    labeled_transcript_from_sources, merge_transcription_context, valid_sources_for_processing,
    SourceTranscriptInput,
};
use clovy_lib::domain::types::{DictionaryEntryDto, RecordingSource};
use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use tempfile::tempdir;

#[test]
fn labeled_transcript_keeps_microphone_and_system_sections() {
    let transcript = labeled_transcript_from_sources(&[
        SourceTranscriptInput {
            source: "microphone".to_string(),
            text: "My action item is to follow up.".to_string(),
            valid: true,
            warning: None,
            start_ms: Some(2_000),
            end_ms: Some(3_000),
            turn_index: Some(1),
            recorded_silence: false,
        },
        SourceTranscriptInput {
            source: "system".to_string(),
            text: "The deadline is Friday.".to_string(),
            valid: true,
            warning: None,
            start_ms: Some(1_000),
            end_ms: Some(1_800),
            turn_index: Some(0),
            recorded_silence: false,
        },
    ]);

    assert_eq!(
        transcript,
        "System: The deadline is Friday.\nMicrophone: My action item is to follow up."
    );
}

#[test]
fn processing_uses_only_valid_source_transcripts() {
    let sources = valid_sources_for_processing(vec![
        SourceTranscriptInput {
            source: "microphone".to_string(),
            text: "valid mic text".to_string(),
            valid: true,
            warning: None,
            start_ms: None,
            end_ms: None,
            turn_index: None,
            recorded_silence: false,
        },
        SourceTranscriptInput {
            source: "system".to_string(),
            text: "invalid system text".to_string(),
            valid: false,
            warning: Some("System audio was silent.".to_string()),
            start_ms: None,
            end_ms: None,
            turn_index: None,
            recorded_silence: false,
        },
    ]);

    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].source, "microphone");
}

#[test]
fn coalesces_consecutive_same_source_transcripts_for_display() {
    let sources = coalesce_source_transcripts(vec![
        SourceTranscriptInput {
            source: "system".to_string(),
            text: "First part.".to_string(),
            valid: true,
            warning: None,
            start_ms: Some(1_000),
            end_ms: Some(3_000),
            turn_index: Some(0),
            recorded_silence: false,
        },
        SourceTranscriptInput {
            source: "system".to_string(),
            text: "Second part.".to_string(),
            valid: true,
            warning: None,
            start_ms: Some(4_000),
            end_ms: Some(5_000),
            turn_index: Some(1),
            recorded_silence: false,
        },
        SourceTranscriptInput {
            source: "microphone".to_string(),
            text: "Reply.".to_string(),
            valid: true,
            warning: None,
            start_ms: Some(7_000),
            end_ms: Some(8_000),
            turn_index: Some(2),
            recorded_silence: false,
        },
    ]);

    assert_eq!(sources.len(), 2);
    assert_eq!(sources[0].source, "system");
    assert_eq!(sources[0].text, "First part. Second part.");
    assert_eq!(sources[0].start_ms, Some(1_000));
    assert_eq!(sources[0].end_ms, Some(5_000));
    assert_eq!(sources[0].turn_index, Some(0));
    assert_eq!(sources[1].turn_index, Some(1));
}

#[test]
fn builds_transcription_context_from_previous_turns() {
    let context = build_transcription_context(&[
        SourceTranscriptInput {
            source: "system".to_string(),
            text: "No, al final lo puedes hacer con Planeta Azul.".to_string(),
            valid: true,
            warning: None,
            start_ms: Some(1_000),
            end_ms: Some(5_000),
            turn_index: Some(0),
            recorded_silence: false,
        },
        SourceTranscriptInput {
            source: "microphone".to_string(),
            text: "Con Planeta Magic.".to_string(),
            valid: true,
            warning: None,
            start_ms: Some(6_000),
            end_ms: Some(7_000),
            turn_index: Some(1),
            recorded_silence: false,
        },
    ])
    .expect("context should be built");

    assert!(context.contains("Previous transcript context"));
    assert!(context.contains("System: No, al final"));
    assert!(context.contains("Microphone: Con Planeta Magic."));
    assert!(context.contains("Preserve the spoken language"));
}

fn dictionary_entries(phrases: &[&str]) -> Vec<DictionaryEntryDto> {
    phrases
        .iter()
        .enumerate()
        .map(|(index, phrase)| DictionaryEntryDto {
            id: format!("entry-{index}"),
            phrase: phrase.to_string(),
            created_at: "2026-05-26T00:00:00Z".to_string(),
            updated_at: "2026-05-26T00:00:00Z".to_string(),
        })
        .collect()
}

fn asr_hint(phrases: &[&str]) -> Option<String> {
    build_transcription_dictionary(&dictionary_entries(phrases))
        .and_then(|dictionary| dictionary.asr_hint)
}

#[test]
fn asr_hint_joins_whole_trimmed_terms_in_dictionary_order() {
    assert_eq!(
        asr_hint(&[" Jane Doe ", "  ", "Clovy", "OSS"]).as_deref(),
        Some("Jane Doe, Clovy, OSS")
    );
    assert_eq!(asr_hint(&[]), None);
}

#[test]
fn asr_hint_counts_utf8_bytes_and_separators_against_its_200_byte_budget() {
    let first = "a".repeat(100);
    // 100 + ", " + 98 is exactly 200 bytes: the term is kept.
    let exact = "b".repeat(98);
    assert_eq!(
        asr_hint(&[&first, &exact]),
        Some(format!("{first}, {exact}"))
    );
    // One byte over: the whole term is skipped, never cut.
    let over = "b".repeat(99);
    assert_eq!(asr_hint(&[&first, &over]), Some(first.clone()));
    // 102 characters but 202 bytes: two-byte characters count twice.
    let accented = "é".repeat(60);
    let umlauts = "ü".repeat(40);
    assert_eq!(asr_hint(&[&accented, &umlauts]), Some(accented.clone()));
}

#[test]
fn asr_hint_skips_a_term_that_does_not_fit_and_keeps_trying_later_ones() {
    let filler = "f".repeat(190);
    assert_eq!(
        asr_hint(&[&filler, "Kubernetes", "OSS"]),
        Some(format!("{filler}, OSS"))
    );

    let oversized = "x".repeat(201);
    assert_eq!(asr_hint(&[&oversized, "Clovy"]).as_deref(), Some("Clovy"));
    // No term fits: no hint at all, but correction still gets the full dictionary.
    let dictionary = build_transcription_dictionary(&dictionary_entries(&[&oversized]))
        .expect("a dictionary with one term");
    assert_eq!(dictionary.asr_hint, None);
    assert!(dictionary.context.contains(&oversized));
}

#[test]
fn merges_dictionary_and_previous_transcription_context() {
    let merged = merge_transcription_context(
        Some("Custom dictionary terms:\n- OSS"),
        Some("Previous transcript context:\nMicrophone: OSS"),
    )
    .expect("merged context");

    assert!(merged.contains("Custom dictionary terms"));
    assert!(merged.contains("Previous transcript context"));
}

/// Write a mono 16kHz 16-bit PCM WAV whose samples alternate `±amplitude`
/// inside `[active_start_ms, active_end_ms)` and are silent elsewhere, so turn
/// detection sees a clear active region against a silent floor.
fn write_active_region_wav(
    path: &Path,
    amplitude: i16,
    duration_ms: u32,
    active_start_ms: u32,
    active_end_ms: u32,
) {
    let spec = WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec).expect("wav writer");
    let samples = (spec.sample_rate as f32 * (duration_ms as f32 / 1000.0)) as usize;
    let active_start = (spec.sample_rate as f32 * (active_start_ms as f32 / 1000.0)) as usize;
    let active_end = (spec.sample_rate as f32 * (active_end_ms as f32 / 1000.0)) as usize;
    for i in 0..samples {
        let sample = if i >= active_start && i < active_end {
            if i % 2 == 0 {
                amplitude
            } else {
                -amplitude
            }
        } else {
            0
        };
        writer.write_sample(sample).expect("sample write");
    }
    writer.finalize().expect("wav finalize");
}

fn patch_le_u32(path: &Path, offset: u64, value: u32) {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open wav for header patch");
    file.seek(SeekFrom::Start(offset))
        .expect("seek header field");
    file.write_all(&value.to_le_bytes())
        .expect("write header field");
    file.flush().expect("flush header field");
}

fn wav_duration_ms(path: &Path) -> i64 {
    let reader = WavReader::open(path).expect("readable wav");
    let sample_rate = reader.spec().sample_rate.max(1) as i64;
    (reader.duration() as i64 * 1000) / sample_rate
}

/// End-to-end pin for the stale-header system-source path fixed in PR #570: a
/// microphone WAV plus a system WAV whose RIFF/`data` header was left stale by a
/// SIGKILLed capture helper (claims ~1s while ~10s of samples sit on disk) must
/// survive validation and the turn pipeline as a real system lane instead of
/// being silently dropped.
///
/// `process_saved_source_audio` transcribes via `default_turn_transcriber()`
/// (a network call) with no injectable seam, and the full-source fallback
/// (`add_full_source_turns_for_missing_sources`) is private, so this drives the
/// deepest network-free depth: Gate 1 validation plus the real public turn
/// pipeline (`detect_turns` -> `coalesce_turns_for_transcription` -> segment
/// extraction/normalization), asserting the system lane reaches non-empty,
/// transcription-ready audio.
#[test]
fn stale_header_system_source_survives_validation_and_turn_pipeline() {
    let dir = tempdir().expect("tempdir");
    let mic_path = dir.path().join("microphone.wav");
    let system_path = dir.path().join("system.wav");

    // Microphone: an audible burst framed by silence so detection finds a mic turn.
    write_active_region_wav(&mic_path, 18_000, 4_000, 1_000, 3_000);
    // System: ~10s with an audible speech-like region framed by silence.
    write_active_region_wav(&system_path, 18_000, 10_000, 2_000, 8_000);

    // Simulate SIGKILL mid-finalization: rewrite the RIFF size (offset 4) and the
    // `data` chunk size (offset 40 for a canonical mono 16-bit PCM WAV) to claim
    // only ~1s while 10s of samples remain on disk.
    let one_second_data_bytes = 16_000 * 2;
    patch_le_u32(&system_path, 4, 32 + one_second_data_bytes);
    patch_le_u32(&system_path, 40, one_second_data_bytes);

    // Gate 1 (load-bearing): before the header repair the stale WAV reads ~1s
    // against an expected 10s and fails `is_not_truncated`, so the system source
    // is dropped. `validate_audio_artifact` repairs the header first, so the
    // system source now passes validation.
    let system_validation =
        validate_audio_artifact(&system_path, 10_000, AudioValidationConfig::default())
            .expect("validation should run");
    assert_eq!(
        system_validation.actual_duration_ms, 10_000,
        "stale header should be repaired to the true on-disk duration"
    );
    assert!(
        source_audio_passes_validation(RecordingSource::System, &system_validation),
        "repaired system source must pass validation (Gate 1)"
    );

    // Drive the real turn pipeline for the dual-source recording.
    let sources: Vec<(String, String, PathBuf)> = vec![
        (
            "mic-artifact".to_string(),
            "microphone".to_string(),
            mic_path.clone(),
        ),
        (
            "system-artifact".to_string(),
            "system".to_string(),
            system_path.clone(),
        ),
    ];
    let detection_sources = sources
        .iter()
        .map(|(artifact_id, source, path)| DetectionSource {
            artifact_id: artifact_id.clone(),
            source: source.clone(),
            path: path.clone(),
        })
        .collect::<Vec<_>>();

    let detected = detect_turns(&detection_sources).expect("turn detection should run");
    // Mirror the private full-source fallback: any source without a detected turn
    // gets a 0..0 sentinel turn covering the whole source recording.
    let mut turns = detected;
    for (artifact_id, source, path) in &sources {
        let has_turn = turns
            .iter()
            .any(|turn| turn.artifact_id == *artifact_id && turn.source == *source);
        if !has_turn {
            turns.push(AudioTurn {
                artifact_id: artifact_id.clone(),
                source: source.clone(),
                source_path: path.clone(),
                extraction_start_ms: 0,
                start_ms: 0,
                end_ms: 0,
                turn_index: turns.len() as i64,
            });
        }
    }
    let turns = coalesce_turns_for_transcription(turns);

    assert!(
        turns.iter().any(|turn| turn.source == "microphone"),
        "microphone lane must be present"
    );
    let system_turn = turns
        .iter()
        .find(|turn| turn.source == "system")
        .expect("system lane must be present, not dropped");

    // Extract the system turn exactly as the pipeline does, then normalize it into
    // transcription-ready audio and assert it is a non-empty ~10s segment.
    let segment_dir = dir.path().join("segments");
    std::fs::create_dir_all(&segment_dir).expect("segment dir");
    let covers_full_source = system_turn.end_ms <= system_turn.start_ms;
    let raw_segment_path = if covers_full_source {
        system_turn.source_path.clone()
    } else {
        let segment_path = segment_dir.join("system-turn.wav");
        write_turn_wav(system_turn, &segment_path).expect("system turn extraction");
        segment_path
    };
    let normalized_path = normalize_wav_for_transcription(
        &raw_segment_path,
        &segment_dir.join("system-turn-normalized.wav"),
    )
    .expect("system turn normalization");

    let normalized_duration_ms = wav_duration_ms(&normalized_path);
    assert!(
        normalized_duration_ms >= 5_000,
        "system lane should carry substantial (~10s) audio, got {normalized_duration_ms}ms"
    );
}
