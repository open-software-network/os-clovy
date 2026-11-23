# JUN-334 note transcription latency

> **Historical measurements (2026-10-28).** The measurements below predate
> the direct-provider cut in ADR-0073. They preserve the original fixture and
> observations and do not prove the current transport or native behavior.


## Measurement origins

The benchmark clock starts after recording finalization, synthetic fixture creation, database setup, and observer readiness. It immediately precedes the call to the existing three-argument `finish_recording_session` handoff. All benchmark fields therefore measure post-finalization orchestration. They do not include the time spent finalizing a recording after the user presses Done.

Production telemetry has a different origin. Its clock starts at the first statement of the public `finish_recording` command, before repository lookup and finalization. Done-relative checkpoints are useful for production diagnosis, but they must not be compared directly with the handoff-relative benchmark values below.

## Environment and revisions

Both runs used the same Mac mini and local toolchain:

- macOS 26.5.2 (build 25F84), Darwin 25.5.0, arm64
- `rustc 1.96.1 (31fca3adb 2026-06-26)`, host `aarch64-apple-darwin`, LLVM 22.1.2
- Cargo release profile, locked dependencies, one benchmark test thread, and a four-worker Tokio test runtime
- Baseline log completed 2026-07-15 12:06:47 +0800
- Feature log completed 2026-07-15 15:39:20 +0800 (2026-07-15 07:39:20 UTC)

Revisions:

- Frozen production baseline: `06f4925ebba8947ae4197887dcf5d9dbba697a16`
- Test-only benchmark source: `68642f6185dd93af14a1892131f3b5e749b533d4`
- Baseline label: `06f4925e + test-only harness`
- Feature revision, short revision, and label: `961606645af33dec07ba0faacf94f06ba397c93d`, `96160664`, `96160664 feature`

Before the feature run, the harness at its historical path,
`src-tauri/src/commands/transcription_benchmark.rs`, and its historical
`benchmark-transcription-latency` Make target were verified byte-for-byte
unchanged from the benchmark source commit. After measurement, the current
harness moved to `src-tauri/src/commands/note_transcription_benchmark.rs` and
received glossary-compliance identifier renames. Those later changes affect
terminology only; they do not change fixture generation, timing origins,
observation behavior, sample selection, or median calculations.

## Benchmark method

The ignored command-layer benchmark uses finalized 48 kHz stereo WAV fixtures with distinct Microphone and System signals. It exercises real Turn detection, WAV preparation, the bounded note transcription scheduler, file-backed SQLite persistence, note generation orchestration, and the Ready transition. The four cases are Microphone-plus-System recording sessions of 1, 5, and 10 minutes and a 5-minute Microphone-only control.

Each case has one warm-up, sample `0`, followed by five measured runs, samples `1` to `5`. Every reported median sorts the five measured values for that field independently and selects index `len / 2`. The warm-up is preserved in the raw evidence but excluded from medians.

A loopback Clovy API records request arrival before draining the request body. It returns a deterministic transcript after 100 ms, a dictation-cleanup response after 25 ms, and a generation response without an added delay. A database observer reads checkpoints, the first succeeded transcript joined to its audio artifact, and note status in one transaction every 5 ms. Consequently, `handoffToFirstPersistedMs` is observer detection time, not the exact commit timestamp. It has one 5 ms polling interval of deliberate sampling quantization plus query and runtime scheduling, so it can be later than both the native commit and an independently observed generation request. Fields are observed and medianed independently, so their median columns do not establish within-run event ordering. The fake generation endpoint characterizes orchestration only and is not production model latency.

The feature checkpoint keeps the legacy `turnWavExtractionDurationMs`/`durationMs` field equal to active preparation time. `activePreparationDurationMs` sums time spent doing preparation work. `producerWallDurationMs` measures the producer's elapsed lifetime, including waiting to send into the capacity-two channel while provider work is active. Producer wall time is intentionally not substituted for active preparation and is not directly comparable with the baseline's eager extraction duration.

## Baseline samples: 06f4925e + test-only harness

All values are milliseconds. Blank cells are JSON `null`. The table preserves all 24 raw records and all four emitted median records.

```csv
revisionLabel,case,sample,handoffToValidationMs,handoffToDequeuedMs,handoffToDetectionCompleteMs,detectionDurationMs,turnWavExtractionDurationMs,activePreparationDurationMs,producerWallDurationMs,handoffToFirstRequestMs,handoffToFirstPersistedMs,handoffToTranscriptionCompleteMs,handoffToReadyMs
"06f4925e + test-only harness","dual-1m","0",135,134,275,136,132,,,407,545,1089,1101
"06f4925e + test-only harness","dual-1m","1",140,137,250,111,132,,,383,515,1055,1064
"06f4925e + test-only harness","dual-1m","2",139,138,256,111,129,,,382,515,1050,1058
"06f4925e + test-only harness","dual-1m","3",137,135,253,111,130,,,379,510,1061,1074
"06f4925e + test-only harness","dual-1m","4",137,137,252,110,130,,,381,515,1046,1054
"06f4925e + test-only harness","dual-1m","5",143,138,253,112,129,,,383,518,1060,1066
"06f4925e + test-only harness","dual-1m","median",139,137,253,111,130,,,382,515,1055,1064
"06f4925e + test-only harness","dual-5m","0",655,650,1219,567,653,,,1874,2008,5274,5282
"06f4925e + test-only harness","dual-5m","1",654,649,1200,546,642,,,1841,1973,5257,5265
"06f4925e + test-only harness","dual-5m","2",652,650,1204,547,648,,,1848,1979,5210,5220
"06f4925e + test-only harness","dual-5m","3",651,650,1205,547,648,,,1849,1985,5254,5264
"06f4925e + test-only harness","dual-5m","4",653,652,1212,551,648,,,1855,1987,5289,5299
"06f4925e + test-only harness","dual-5m","5",655,651,1205,550,646,,,1850,1981,5269,5280
"06f4925e + test-only harness","dual-5m","median",653,650,1205,547,648,,,1849,1981,5257,5265
"06f4925e + test-only harness","dual-10m","0",1284,1284,2385,1093,1300,,,3680,3816,10480,10489
"06f4925e + test-only harness","dual-10m","1",1299,1296,2400,1097,1288,,,3685,3819,10535,10544
"06f4925e + test-only harness","dual-10m","2",1299,1299,2405,1103,1421,,,3828,3961,10547,10555
"06f4925e + test-only harness","dual-10m","3",1285,1287,2390,1100,1296,,,3686,3821,10420,10429
"06f4925e + test-only harness","dual-10m","4",1290,1287,2382,1090,1284,,,3664,3800,10483,10489
"06f4925e + test-only harness","dual-10m","5",1290,1286,2395,1100,1294,,,3683,3816,10508,10519
"06f4925e + test-only harness","dual-10m","median",1290,1287,2395,1100,1294,,,3685,3819,10508,10519
"06f4925e + test-only harness","mic-5m-control","0",329,324,,,,,,576,1729,1727,1736
"06f4925e + test-only harness","mic-5m-control","1",330,328,,,,,,578,1726,1727,1739
"06f4925e + test-only harness","mic-5m-control","2",332,328,,,,,,576,1727,1718,1727
"06f4925e + test-only harness","mic-5m-control","3",330,325,,,,,,574,1718,1713,1725
"06f4925e + test-only harness","mic-5m-control","4",330,329,,,,,,576,1711,1709,1717
"06f4925e + test-only harness","mic-5m-control","5",341,339,,,,,,600,1761,1753,1761
"06f4925e + test-only harness","mic-5m-control","median",330,328,,,,,,576,1726,1718,1727
```

The pre-change applicability gate used the five-minute dual-source medians: eager turn-WAV extraction was `648 ms` and handoff to first request was `1849 ms`. The ratio was `648 / 1849 = 0.350460`, or 35.0460%, above the locked 20% threshold. Result: PASS.

## Feature samples

All values are milliseconds. Blank cells are JSON `null`. The table preserves all 24 raw records and all four emitted median records.

```csv
revisionLabel,case,sample,handoffToValidationMs,handoffToDequeuedMs,handoffToDetectionCompleteMs,detectionDurationMs,turnWavExtractionDurationMs,activePreparationDurationMs,producerWallDurationMs,handoffToFirstRequestMs,handoffToFirstPersistedMs,handoffToTranscriptionCompleteMs,handoffToReadyMs
"96160664 feature","dual-1m","0",134,134,253,111,61,61,419,254,385,940,950
"96160664 feature","dual-1m","1",145,145,257,110,62,62,415,263,398,951,962
"96160664 feature","dual-1m","2",143,141,253,108,64,64,417,256,391,943,952
"96160664 feature","dual-1m","3",141,141,253,107,63,63,417,255,392,943,953
"96160664 feature","dual-1m","4",141,137,251,110,63,63,416,254,390,945,955
"96160664 feature","dual-1m","5",144,144,262,110,62,62,416,262,397,947,958
"96160664 feature","dual-1m","median",143,141,253,110,63,63,416,256,392,945,955
"96160664 feature","dual-5m","0",698,694,1245,543,445,445,3139,1245,1379,4659,4670
"96160664 feature","dual-5m","1",672,672,1223,543,463,463,3148,1223,1359,4656,4664
"96160664 feature","dual-5m","2",707,705,1256,544,456,456,3144,1257,1388,4677,4689
"96160664 feature","dual-5m","3",655,654,1199,541,463,463,3148,1203,1338,4627,4638
"96160664 feature","dual-5m","4",685,679,1221,540,458,458,3175,1226,1362,4677,4683
"96160664 feature","dual-5m","5",645,646,1189,536,455,455,3155,1189,1325,4625,4635
"96160664 feature","dual-5m","median",672,672,1221,541,458,458,3148,1223,1359,4656,4664
"96160664 feature","dual-10m","0",1400,1399,2478,1073,964,964,6562,2479,2617,9329,9338
"96160664 feature","dual-10m","1",1343,1343,2423,1078,945,945,6577,2429,2562,9297,9308
"96160664 feature","dual-10m","2",1336,1336,2416,1074,967,967,6584,2418,2556,9283,9296
"96160664 feature","dual-10m","3",1319,1315,2391,1073,953,953,6565,2395,2530,9233,9245
"96160664 feature","dual-10m","4",1381,1377,2459,1078,838,838,6478,2462,2598,9223,9232
"96160664 feature","dual-10m","5",1310,1310,2384,1069,955,955,6578,2387,2522,9252,9264
"96160664 feature","dual-10m","median",1336,1336,2416,1074,953,953,6577,2418,2556,9252,9264
"96160664 feature","mic-5m-control","0",347,343,,,,,,606,1756,1754,1762
"96160664 feature","mic-5m-control","1",338,337,,,,,,595,1742,1744,1755
"96160664 feature","mic-5m-control","2",352,351,,,,,,607,1751,1753,1764
"96160664 feature","mic-5m-control","3",334,334,,,,,,589,1728,1724,1734
"96160664 feature","mic-5m-control","4",332,329,,,,,,592,1733,1731,1740
"96160664 feature","mic-5m-control","5",343,343,,,,,,607,1758,1758,1769
"96160664 feature","mic-5m-control","median",338,337,,,,,,595,1742,1744,1755
```

The dual-source feature medians separate active preparation from producer wall time: `63 / 416 ms` for one minute, `458 / 3148 ms` for five minutes, and `953 / 6577 ms` for ten minutes. The long producer wall shows capacity-two backpressure while requests run; the much shorter active value measures preparation work itself.

## Median comparison and gates

The key end-to-end medians are:

| Case | Revision | First request | First persisted | Note transcription complete | Ready |
| --- | --- | ---: | ---: | ---: | ---: |
| dual-1m | baseline | 382 | 515 | 1055 | 1064 |
| dual-1m | feature | 256 | 392 | 945 | 955 |
| dual-5m | baseline | 1849 | 1981 | 5257 | 5265 |
| dual-5m | feature | 1223 | 1359 | 4656 | 4664 |
| dual-10m | baseline | 3685 | 3819 | 10508 | 10519 |
| dual-10m | feature | 2418 | 2556 | 9252 | 9264 |
| mic-5m-control | baseline | 576 | 1726 | 1718 | 1727 |
| mic-5m-control | feature | 595 | 1742 | 1744 | 1755 |

The locked five-minute gates use integer medians without rounding:

| Gate | Exact calculation | Threshold | Result |
| --- | --- | --- | --- |
| First persisted improvement | `(1981 - 1359) / 1981 = 622 / 1981 = 31.3983%` | at least 20%; integer check `62200 >= 39620` | PASS |
| Dual-source completion improvement | `(5257 - 4656) / 5257 = 601 / 5257 = 11.4324%` | at least 10%; integer check `60100 >= 52570` | PASS |
| Microphone-only completion regression | `(1744 - 1718) / 1718 = 26 / 1718 = 1.5134%` | at most 5%; integer check `2600 <= 8590` | PASS |

Equivalently, the locked integer limits were at most `1584 ms` for five-minute first persistence, at most `4731 ms` for dual-source completion, and at most `1803 ms` for microphone-only completion. The feature medians satisfy all three.

The non-gating dual-source cases moved in the same direction. First persistence improved 23.8835% and completion improved 10.4265% at one minute; first persistence improved 33.0715% and completion improved 11.9528% at ten minutes.

## Causal deterministic proofs

The benchmark improvement is backed by deterministic tests at the scheduling boundaries:

- `prepared_turn_matches_existing_audio_and_metadata` compares the prepared PCM samples and metadata with the prior extraction and normalization path.
- `successful_jobs_skip_complete_source_preparation`, `failed_source_prepares_one_lazy_fallback_with_source_operation_id`, `failed_sources_prepare_one_fallback_each`, `echo_trimmed_source_never_prepares_or_transcribes_fallback`, and `valid_cached_turn_suppresses_fallback_after_fresh_failures` prove lazy, once-per-source, normalized fallback behavior without duplicate work.
- `pipeline_starts_first_request_before_later_preparation_finishes` proves note transcription begins while later Turns are still being prepared.
- `streaming_scheduler_never_exceeds_two_provider_calls` and `preparation_report_separates_active_time_from_backpressure` exercise five descriptors and prove provider concurrency two, channel capacity two, fifth-send backpressure, and separate active and wall timings.
- `streaming_scheduler_preserves_logical_spawn_context` and `pipelined_results_are_sorted_after_reverse_completion` preserve the prior context launch points and chronological output under overlap.
- Preparation, sink, channel-close, and cancellation regressions prove started work is drained, the first error is preserved, fallback is skipped after preparation failure, and Turn WAV files outlive blocking/provider work.
- `done_origin_checkpoints_are_monotonic_and_single_shot`, `first_event_timeline_flushes_each_checkpoint_once`, and pipeline failure telemetry tests prove Done-relative checkpoints are monotonic, single-shot, and best-effort.
- `polls newly persisted turns while note transcription remains active` proves the existing App poll renders an exact newly committed Turn while the Note remains Transcribing, without a production React change.

The Task 2 through Task 6 focused suites, the 69-test native processing module, strict Rust Clippy, frontend regression, typecheck, and repository lint passed before this measurement. The final full repository gate is intentionally separate from this benchmark evidence.

## UI visibility bound

The polling behavior below describes the measured JUN-334 revision. JUN-390
later replaced that one-second full-Note poll with native processing progress
events and one full hydration on the terminal `done` event.

The benchmark's first-persisted value detects the native persistence milestone; it is not the exact commit timestamp or a browser paint. In the measured revision, the App refreshed an active selected Note on a one-second interval. Native persistence therefore preceded visible output by **0 to 1 second** plus database, IPC, reducer, and render time. The database observer's 5 ms polling interval separately quantized the benchmarked persistence timestamp; it was not part of the UI polling bound.

The historical App regression started with no saved source turns, changed the next poll response to include one exact saved turn while keeping status `transcribing`, and observed both that text and the live `Transcribing audio` status. JUN-390 replaces that regression with coverage proving intermediate events do not hydrate the Note and `done` hydrates it exactly once.

## Scope and limitations

The synthetic fixtures make real audio DSP and SQLite work repeatable, while the loopback responses isolate desktop scheduling. The fake generation endpoint measures orchestration, not production model latency or generated Note quality. Network variability, production model queueing, recording finalization, and real recording acoustics are outside this benchmark.

The change does not reuse live preview text, increase provider concurrency, add batching, optimize generation, change the one-second UI poll, or persist Microphone-only chunks incrementally. It preserves finalized saved audio as the note transcription source.

This is a desktop-only scheduling change and does not require a Clovy API deploy.

## Reproduction commands

The recorded CSV blocks above retain the historical serialized field
`handoffToTranscriptionCompleteMs`. The terminology-only rename emits
`handoffToNoteTranscriptionCompleteMs` for new runs with identical timing
semantics.

Run the current, glossary-compliant harness without concurrent builds or tests:

```bash
FEATURE_LABEL="$(git rev-parse --short HEAD) feature"
set -o pipefail
JUN334_REVISION_LABEL="$FEATURE_LABEL" \
  make benchmark-note-transcription-latency 2>&1 | tee /tmp/jun334-feature.log

rg -o 'JUN334_BENCHMARK \{.*\}' /tmp/jun334-feature.log
```

Reproduce the frozen baseline in a fresh temporary worktree with the exact test-only benchmark overlay:

```bash
git worktree add /tmp/os-june-jun334-baseline \
  06f4925ebba8947ae4197887dcf5d9dbba697a16
git -C /tmp/os-june-jun334-baseline cherry-pick \
  68642f6185dd93af14a1892131f3b5e749b533d4
JUN334_REVISION_LABEL='06f4925e + test-only harness' \
  make -C /tmp/os-june-jun334-baseline benchmark-transcription-latency
```

The frozen overlay command intentionally uses the historical target supplied by
commit `68642f61`; the current worktree uses
`benchmark-note-transcription-latency`.

The marker must be extracted anywhere on a line because libtest can prefix the first record. A valid log contains exactly 28 markers: iterations `0` through `5` and one median for each of four cases.
