use super::{finish_recording_session, sqlite_connect_options, sqlite_pool_options};
use crate::{
    audio::capture::{FinishedRecording, FinishedSource},
    db::{migrations::run_migrations, repositories::Repositories},
    domain::types::{
        AudioLevelDto, ProcessingStatus, RecordingSessionDto, RecordingSource, RecordingSourceMode,
        RecordingState,
    },
};
use serde::Serialize;
use sqlx::row::Row;
use sqlx_sqlite::SqliteJournalMode;
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{oneshot, Notify},
};

const WARMUP_RUNS: usize = 1;
const MEASURED_RUNS: usize = 5;
const _: () = assert!(MEASURED_RUNS > 0 && MEASURED_RUNS % 2 == 1);

#[derive(Clone, Copy)]
struct BenchmarkCase {
    name: &'static str,
    duration_minutes: u32,
    source_mode: RecordingSourceMode,
}

const CASES: [BenchmarkCase; 4] = [
    BenchmarkCase {
        name: "dual-1m",
        duration_minutes: 1,
        source_mode: RecordingSourceMode::MicrophonePlusSystem,
    },
    BenchmarkCase {
        name: "dual-5m",
        duration_minutes: 5,
        source_mode: RecordingSourceMode::MicrophonePlusSystem,
    },
    BenchmarkCase {
        name: "dual-10m",
        duration_minutes: 10,
        source_mode: RecordingSourceMode::MicrophonePlusSystem,
    },
    BenchmarkCase {
        name: "mic-5m-control",
        duration_minutes: 5,
        source_mode: RecordingSourceMode::MicrophoneOnly,
    },
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkSample {
    revision_label: String,
    case: String,
    iteration: usize,
    handoff_to_validation_ms: i64,
    handoff_to_dequeued_ms: i64,
    handoff_to_detection_complete_ms: Option<i64>,
    detection_duration_ms: Option<i64>,
    turn_wav_extraction_duration_ms: Option<i64>,
    active_preparation_duration_ms: Option<i64>,
    producer_wall_duration_ms: Option<i64>,
    handoff_to_first_request_ms: i64,
    handoff_to_first_persisted_ms: i64,
    handoff_to_note_transcription_complete_ms: i64,
    handoff_to_ready_ms: i64,
}

#[derive(Clone, Default)]
pub(super) struct BenchmarkClock {
    origin: Arc<OnceLock<Instant>>,
}

impl BenchmarkClock {
    pub(super) fn start(&self) {
        self.origin
            .set(Instant::now())
            .expect("benchmark clock starts once");
    }

    pub(super) fn elapsed_ms(&self) -> i64 {
        self.origin
            .get()
            .expect("benchmark clock started")
            .elapsed()
            .as_millis()
            .min(i64::MAX as u128) as i64
    }
}

#[derive(Clone)]
pub(super) struct RequestEvents {
    clock: BenchmarkClock,
    first_note_transcription_ms: Arc<Mutex<Option<i64>>>,
    first_generation_ms: Arc<Mutex<Option<i64>>>,
    changed: Arc<Notify>,
}

impl RequestEvents {
    pub(super) fn new(clock: BenchmarkClock) -> Self {
        Self {
            clock,
            first_note_transcription_ms: Arc::new(Mutex::new(None)),
            first_generation_ms: Arc::new(Mutex::new(None)),
            changed: Arc::new(Notify::new()),
        }
    }

    fn record(&self, path: &str, body: &[u8]) {
        let slot = match path {
            "/openai/v1/audio/transcriptions" => Some(&self.first_note_transcription_ms),
            "/api/v1/chat/completions"
                if serde_json::from_slice::<serde_json::Value>(body)
                    .is_ok_and(|body| body.get("temperature").is_none()) =>
            {
                Some(&self.first_generation_ms)
            }
            _ => None,
        };
        if let Some(slot) = slot {
            let mut value = slot.lock().expect("request event mutex");
            if value.is_none() {
                *value = Some(self.clock.elapsed_ms());
                self.changed.notify_waiters();
            }
        }
    }

    fn first_note_transcription_ms(&self) -> i64 {
        self.first_note_transcription_ms
            .lock()
            .expect("request event mutex")
            .expect("note transcription request observed")
    }

    fn first_generation_ms(&self) -> i64 {
        self.first_generation_ms
            .lock()
            .expect("request event mutex")
            .expect("generation request observed")
    }
}

#[derive(Clone)]
struct BenchmarkObserver {
    clock: BenchmarkClock,
    dequeued_ms: Arc<Mutex<Option<i64>>>,
}

static BENCHMARK_OBSERVERS: LazyLock<Mutex<HashMap<String, BenchmarkObserver>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn register_benchmark_observer(recording_session_id: &str, observer: BenchmarkObserver) {
    BENCHMARK_OBSERVERS
        .lock()
        .expect("benchmark observer mutex")
        .insert(recording_session_id.to_string(), observer);
}

pub(super) fn record_processing_dequeued(recording_session_id: &str) {
    let observer = BENCHMARK_OBSERVERS
        .lock()
        .expect("benchmark observer mutex")
        .get(recording_session_id)
        .cloned();
    if let Some(observer) = observer {
        let mut value = observer.dequeued_ms.lock().expect("dequeue event mutex");
        if value.is_none() {
            *value = Some(observer.clock.elapsed_ms());
        }
    }
}

fn remove_benchmark_observer(recording_session_id: &str) {
    BENCHMARK_OBSERVERS
        .lock()
        .expect("benchmark observer mutex")
        .remove(recording_session_id);
}

pub(super) async fn benchmark_repositories(dir: &tempfile::TempDir) -> Repositories {
    let database_path = dir.path().join("clovy-benchmark.sqlite3");
    let options = sqlite_connect_options(&database_path)
        .expect("SQLite options")
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal);
    let pool = sqlite_pool_options(5)
        .connect_with(options)
        .await
        .expect("benchmark database");
    run_migrations(&pool).await.expect("benchmark migrations");
    Repositories::new(pool)
}

fn write_benchmark_wav(path: &Path, minutes: u32, source: RecordingSource) {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).expect("create benchmark WAV");
    let frames = u64::from(minutes) * 60 * u64::from(spec.sample_rate);
    let frequency = match source {
        RecordingSource::Microphone => 311.0_f32,
        RecordingSource::System => 577.0_f32,
    };
    for frame in 0..frames {
        let cycle_second = (frame / u64::from(spec.sample_rate)) % 12;
        let active = match source {
            RecordingSource::Microphone => cycle_second < 3,
            RecordingSource::System => (6..9).contains(&cycle_second),
        };
        let sample = if active {
            let phase = frame as f32 * frequency * std::f32::consts::TAU / spec.sample_rate as f32;
            (phase.sin() * 8_000.0) as i16
        } else {
            0_i16
        };
        writer.write_sample(sample).expect("left sample");
        writer.write_sample(sample).expect("right sample");
    }
    writer.finalize().expect("finalize benchmark WAV");
}

fn prepare_case_fixtures(root: &Path, case: BenchmarkCase) -> Vec<(RecordingSource, PathBuf)> {
    let microphone = root.join(format!("{}-microphone.wav", case.name));
    write_benchmark_wav(
        &microphone,
        case.duration_minutes,
        RecordingSource::Microphone,
    );
    let mut fixtures = vec![(RecordingSource::Microphone, microphone)];
    if case.source_mode == RecordingSourceMode::MicrophonePlusSystem {
        let system = root.join(format!("{}-system.wav", case.name));
        write_benchmark_wav(&system, case.duration_minutes, RecordingSource::System);
        fixtures.push((RecordingSource::System, system));
    }
    fixtures
}

async fn read_http_request(stream: &mut TcpStream, events: &RequestEvents) -> (String, Vec<u8>) {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 8_192];
        let read = stream.read(&mut chunk).await.expect("read request");
        assert!(read > 0, "connection closed before headers");
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let path = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .expect("request path")
        .to_string();
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().expect("content length"))
        })
        .unwrap_or(0);
    while bytes.len() - header_end < content_length {
        let mut chunk = [0_u8; 32_768];
        let read = stream.read(&mut chunk).await.expect("read request body");
        assert!(read > 0, "connection closed before request body");
        bytes.extend_from_slice(&chunk[..read]);
    }
    let body = bytes[header_end..header_end + content_length].to_vec();
    events.record(&path, &body);
    (path, body)
}

async fn handle_fake_request(mut stream: TcpStream, events: RequestEvents) {
    let (path, _body) = read_http_request(&mut stream, &events).await;
    let (delay, body) = match path.as_str() {
        "/openai/v1/audio/transcriptions" => (
            Duration::from_millis(100),
            r#"{"text":"benchmark transcript","language":"en","segments":[{"start":0.0,"end":1.0,"text":"benchmark transcript"}]}"#,
        ),
        "/api/v1/chat/completions" => (
            Duration::from_millis(25),
            r#"{"choices":[{"message":{"role":"assistant","content":"Benchmark note"}}]}"#,
        ),
        _ => panic!("unexpected benchmark route: {path}"),
    };
    tokio::time::sleep(delay).await;
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .await
        .expect("write response");
    stream.shutdown().await.expect("close response");
}

pub(super) fn configure_fake_note_provider_keys() {
    use crate::providers::BuiltinProvider;
    let store = crate::llm::secrets::store();
    store
        .set(BuiltinProvider::Groq.secret_account(), "fixture-groq-key")
        .unwrap();
    store
        .set(
            BuiltinProvider::OpenRouter.secret_account(),
            "fixture-openrouter-key",
        )
        .unwrap();
    let mut settings = crate::providers::default_settings_for_tests();
    settings.provider_api_keys_configured.groq = true;
    settings.provider_api_keys_configured.openrouter = true;
    settings.llm_usage.notes = crate::llm::registry::ProviderRef::OpenRouter;
    crate::providers::replace_current_settings_for_tests(settings);
}

pub(super) async fn spawn_fake_note_providers(
    events: RequestEvents,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback API");
    let address = listener.local_addr().expect("loopback address");
    let handle = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(handle_fake_request(stream, events.clone()));
        }
    });
    (address, handle)
}

#[derive(Default)]
struct DatabaseObservation {
    handoff_to_validation_ms: Option<i64>,
    handoff_to_detection_complete_ms: Option<i64>,
    detection_duration_ms: Option<i64>,
    turn_wav_extraction_duration_ms: Option<i64>,
    active_preparation_duration_ms: Option<i64>,
    producer_wall_duration_ms: Option<i64>,
    handoff_to_first_persisted_ms: Option<i64>,
    handoff_to_ready_ms: Option<i64>,
}

fn checkpoint_details(row: &sqlx_sqlite::SqliteRow) -> serde_json::Value {
    row.try_get::<Option<String>, _>("details")
        .expect("checkpoint details")
        .and_then(|details| serde_json::from_str(&details).ok())
        .unwrap_or(serde_json::Value::Null)
}

async fn observe_database(
    repos: Repositories,
    recording_session_id: String,
    note_id: String,
    clock: BenchmarkClock,
    ready: oneshot::Sender<()>,
    start: oneshot::Receiver<()>,
) -> DatabaseObservation {
    ready.send(()).expect("signal database observer readiness");
    start.await.expect("start database observer");
    let mut observation = DatabaseObservation::default();
    loop {
        let mut transaction = repos.pool.begin().await.expect("observer transaction");
        let checkpoint_rows = sqlx::query::query(
            "SELECT kind, details
             FROM recording_checkpoints
             WHERE recording_session_id = ?
             ORDER BY created_at ASC, rowid ASC",
        )
        .bind(&recording_session_id)
        .fetch_all(&mut *transaction)
        .await
        .expect("observer checkpoints");
        let transcript_row = sqlx::query::query(
            "SELECT EXISTS(
               SELECT 1
               FROM transcripts t
               JOIN audio_artifacts a ON a.id = t.audio_artifact_id
               WHERE a.recording_session_id = ?
                 AND t.status = 'succeeded'
             ) AS present",
        )
        .bind(&recording_session_id)
        .fetch_one(&mut *transaction)
        .await
        .expect("observer transcript");
        let note_row =
            sqlx::query::query("SELECT processing_status, last_error FROM notes WHERE id = ?")
                .bind(&note_id)
                .fetch_one(&mut *transaction)
                .await
                .expect("observer note");
        transaction
            .commit()
            .await
            .expect("observer transaction commit");

        let observed_ms = clock.elapsed_ms();
        for row in &checkpoint_rows {
            let kind: String = row.try_get("kind").expect("checkpoint kind");
            match kind.as_str() {
                "audio_validation" if observation.handoff_to_validation_ms.is_none() => {
                    observation.handoff_to_validation_ms = Some(observed_ms);
                }
                "turn_detection" if observation.handoff_to_detection_complete_ms.is_none() => {
                    let details = checkpoint_details(row);
                    observation.handoff_to_detection_complete_ms = Some(observed_ms);
                    observation.detection_duration_ms = details["durationMs"].as_i64();
                }
                "turn_wav_extraction" if observation.turn_wav_extraction_duration_ms.is_none() => {
                    let details = checkpoint_details(row);
                    observation.turn_wav_extraction_duration_ms = details["durationMs"].as_i64();
                    observation.active_preparation_duration_ms =
                        details["activePreparationDurationMs"].as_i64();
                    observation.producer_wall_duration_ms =
                        details["producerWallDurationMs"].as_i64();
                }
                _ => {}
            }
        }
        let transcript_present: i64 = transcript_row.try_get("present").expect("transcript flag");
        if transcript_present != 0 && observation.handoff_to_first_persisted_ms.is_none() {
            observation.handoff_to_first_persisted_ms = Some(observed_ms);
        }
        let status: String = note_row.try_get("processing_status").expect("note status");
        if status == ProcessingStatus::Failed.as_db() {
            let last_error: Option<String> = note_row.try_get("last_error").expect("note error");
            panic!(
                "benchmark processing reached status failed: {}",
                last_error.as_deref().unwrap_or("unknown error")
            );
        }
        if status == ProcessingStatus::Ready.as_db() {
            observation.handoff_to_ready_ms.get_or_insert(observed_ms);
            return observation;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn benchmark_timeout(case: BenchmarkCase) -> Duration {
    Duration::from_secs(180 + u64::from(case.duration_minutes) * 120)
}

async fn run_benchmark_iteration(
    revision_label: &str,
    case: BenchmarkCase,
    iteration: usize,
    fixtures: &[(RecordingSource, PathBuf)],
) -> BenchmarkSample {
    let dir = tempfile::tempdir().expect("iteration tempdir");
    let repos = benchmark_repositories(&dir).await;
    let note = repos
        .create_note("default", None)
        .await
        .expect("benchmark note");
    let recording_session_id = format!("jun334-{}-{iteration}", case.name);
    let primary_path = fixtures
        .iter()
        .find(|(source, _)| *source == RecordingSource::Microphone)
        .map(|(_, path)| path)
        .expect("microphone fixture");
    let partial_path = primary_path.with_extension("partial.wav");
    repos
        .create_recording_session(
            &note.id,
            &recording_session_id,
            case.source_mode,
            &partial_path.to_string_lossy(),
            &primary_path.to_string_lossy(),
            Some("JUN-334 benchmark".to_string()),
        )
        .await
        .expect("benchmark recording session");
    for (source, path) in fixtures {
        let source_partial_path = path.with_extension("partial.wav");
        repos
            .create_pending_source_artifact(
                &note.id,
                &recording_session_id,
                source.as_db(),
                &source_partial_path.to_string_lossy(),
                &path.to_string_lossy(),
            )
            .await
            .expect("benchmark source artifact");
    }
    let elapsed_ms = i64::from(case.duration_minutes) * 60_000;
    let finished = FinishedRecording {
        session_id: recording_session_id.clone(),
        note_id: note.id.clone(),
        source_mode: case.source_mode,
        final_path: primary_path.clone(),
        sources: fixtures
            .iter()
            .map(|(source, path)| FinishedSource {
                source: *source,
                final_path: path.clone(),
                elapsed_ms,
                dropped_samples: 0,
                capture_issue: None,
                failure: None,
            })
            .collect(),
        elapsed_ms,
        recording: RecordingSessionDto {
            id: recording_session_id.clone(),
            note_id: note.id.clone(),
            source_mode: case.source_mode,
            state: RecordingState::Validating,
            started_at: "JUN-334 benchmark".to_string(),
            elapsed_ms,
            device_label: Some("JUN-334 benchmark".to_string()),
            level: AudioLevelDto::default(),
            live_preview_enabled: false,
            sources: Vec::new(),
            warnings: Vec::new(),
        },
    };

    let clock = BenchmarkClock::default();
    let request_events = RequestEvents::new(clock.clone());
    let (address, api_handle) = spawn_fake_note_providers(request_events.clone()).await;
    std::env::set_var(
        "CLOVY_TEST_NOTE_PROVIDER_ORIGIN",
        format!("http://{address}"),
    );
    configure_fake_note_provider_keys();
    let dequeued_ms = Arc::new(Mutex::new(None));
    register_benchmark_observer(
        &recording_session_id,
        BenchmarkObserver {
            clock: clock.clone(),
            dequeued_ms: dequeued_ms.clone(),
        },
    );
    let (observer_ready_tx, observer_ready_rx) = oneshot::channel();
    let (observer_start_tx, observer_start_rx) = oneshot::channel();
    let database_observer = tokio::spawn(observe_database(
        repos.clone(),
        recording_session_id.clone(),
        note.id.clone(),
        clock.clone(),
        observer_ready_tx,
        observer_start_rx,
    ));

    observer_ready_rx
        .await
        .expect("database observer should become ready");
    clock.start();
    observer_start_tx
        .send(())
        .expect("start database observer polling");
    let observation = tokio::time::timeout(benchmark_timeout(case), async {
        let response = finish_recording_session(&repos, finished, Instant::now())
            .await
            .expect("finish benchmark recording");
        assert!(
            response.processing_started,
            "benchmark processing should start"
        );
        database_observer.await.expect("database observer task")
    })
    .await
    .expect("benchmark iteration timeout");

    api_handle.abort();
    let _ = api_handle.await;
    remove_benchmark_observer(&recording_session_id);
    let handoff_to_dequeued_ms = dequeued_ms
        .lock()
        .expect("dequeue event mutex")
        .expect("processing dequeue observed");

    BenchmarkSample {
        revision_label: revision_label.to_string(),
        case: case.name.to_string(),
        iteration,
        handoff_to_validation_ms: observation
            .handoff_to_validation_ms
            .expect("audio validation checkpoint observed"),
        handoff_to_dequeued_ms,
        handoff_to_detection_complete_ms: observation.handoff_to_detection_complete_ms,
        detection_duration_ms: observation.detection_duration_ms,
        turn_wav_extraction_duration_ms: observation.turn_wav_extraction_duration_ms,
        active_preparation_duration_ms: observation.active_preparation_duration_ms,
        producer_wall_duration_ms: observation.producer_wall_duration_ms,
        handoff_to_first_request_ms: request_events.first_note_transcription_ms(),
        handoff_to_first_persisted_ms: observation
            .handoff_to_first_persisted_ms
            .expect("persisted transcript observed"),
        handoff_to_note_transcription_complete_ms: request_events.first_generation_ms(),
        handoff_to_ready_ms: observation
            .handoff_to_ready_ms
            .expect("ready status observed"),
    }
}

fn median(mut values: Vec<i64>) -> i64 {
    assert!(!values.is_empty(), "median needs values");
    assert!(
        values.len() % 2 == 1,
        "median needs an odd number of values"
    );
    values.sort_unstable();
    values[values.len() / 2]
}

fn optional_median(values: Vec<Option<i64>>) -> Option<i64> {
    let sample_count = values.len();
    let present = values.into_iter().flatten().collect::<Vec<_>>();
    assert!(
        present.is_empty() || present.len() == sample_count,
        "optional median needs all or none of the sample values"
    );
    (!present.is_empty()).then(|| median(present))
}

#[test]
fn median_selects_the_middle_of_five_unsorted_values() {
    assert_eq!(median(vec![13, 2, 8, 5, 21]), 8);
}

#[test]
#[should_panic(expected = "median needs an odd number of values")]
fn median_rejects_even_sample_counts() {
    median(vec![1, 2]);
}

#[test]
fn optional_median_accepts_all_missing_values() {
    assert_eq!(optional_median(vec![None; MEASURED_RUNS]), None);
}

#[test]
#[should_panic(expected = "optional median needs all or none of the sample values")]
fn optional_median_rejects_partial_samples() {
    optional_median(vec![Some(1), None, Some(3), Some(4), Some(5)]);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkMedian<'a> {
    revision_label: &'a str,
    case: &'a str,
    aggregate: &'static str,
    handoff_to_validation_ms: i64,
    handoff_to_dequeued_ms: i64,
    handoff_to_detection_complete_ms: Option<i64>,
    detection_duration_ms: Option<i64>,
    turn_wav_extraction_duration_ms: Option<i64>,
    active_preparation_duration_ms: Option<i64>,
    producer_wall_duration_ms: Option<i64>,
    handoff_to_first_request_ms: i64,
    handoff_to_first_persisted_ms: i64,
    handoff_to_note_transcription_complete_ms: i64,
    handoff_to_ready_ms: i64,
}

fn print_case_medians(revision_label: &str, case: &str, samples: &[BenchmarkSample]) {
    let medians = BenchmarkMedian {
        revision_label,
        case,
        aggregate: "median",
        handoff_to_validation_ms: median(
            samples
                .iter()
                .map(|sample| sample.handoff_to_validation_ms)
                .collect(),
        ),
        handoff_to_dequeued_ms: median(
            samples
                .iter()
                .map(|sample| sample.handoff_to_dequeued_ms)
                .collect(),
        ),
        handoff_to_detection_complete_ms: optional_median(
            samples
                .iter()
                .map(|sample| sample.handoff_to_detection_complete_ms)
                .collect(),
        ),
        detection_duration_ms: optional_median(
            samples
                .iter()
                .map(|sample| sample.detection_duration_ms)
                .collect(),
        ),
        turn_wav_extraction_duration_ms: optional_median(
            samples
                .iter()
                .map(|sample| sample.turn_wav_extraction_duration_ms)
                .collect(),
        ),
        active_preparation_duration_ms: optional_median(
            samples
                .iter()
                .map(|sample| sample.active_preparation_duration_ms)
                .collect(),
        ),
        producer_wall_duration_ms: optional_median(
            samples
                .iter()
                .map(|sample| sample.producer_wall_duration_ms)
                .collect(),
        ),
        handoff_to_first_request_ms: median(
            samples
                .iter()
                .map(|sample| sample.handoff_to_first_request_ms)
                .collect(),
        ),
        handoff_to_first_persisted_ms: median(
            samples
                .iter()
                .map(|sample| sample.handoff_to_first_persisted_ms)
                .collect(),
        ),
        handoff_to_note_transcription_complete_ms: median(
            samples
                .iter()
                .map(|sample| sample.handoff_to_note_transcription_complete_ms)
                .collect(),
        ),
        handoff_to_ready_ms: median(
            samples
                .iter()
                .map(|sample| sample.handoff_to_ready_ms)
                .collect(),
        ),
    };
    println!(
        "JUN334_BENCHMARK {}",
        serde_json::to_string(&medians).expect("serialize benchmark medians")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "release-only JUN-334 benchmark"]
async fn benchmark_post_finalization_note_transcription_latency() {
    let revision_label =
        std::env::var("JUN334_REVISION_LABEL").unwrap_or_else(|_| "unlabeled".to_string());
    let root = tempfile::tempdir().expect("benchmark tempdir");
    let fixtures = CASES
        .into_iter()
        .map(|case| (case, prepare_case_fixtures(root.path(), case)))
        .collect::<Vec<_>>();
    // This release-only harness uses a dynamic loopback API address. Its Make target is the
    // supported entry point and isolates the exact ignored test with one test-harness thread,
    // so these process-global values cannot race unrelated tests.
    let previous = ["CLOVY_TEST_NOTE_PROVIDER_ORIGIN"].map(|name| (name, std::env::var_os(name)));

    for (case, case_fixtures) in fixtures {
        let mut measured = Vec::new();
        for iteration in 0..(WARMUP_RUNS + MEASURED_RUNS) {
            let sample =
                run_benchmark_iteration(&revision_label, case, iteration, &case_fixtures).await;
            println!(
                "JUN334_BENCHMARK {}",
                serde_json::to_string(&sample).expect("serialize benchmark sample")
            );
            if iteration >= WARMUP_RUNS {
                measured.push(sample);
            }
        }
        print_case_medians(&revision_label, case.name, &measured);
    }

    for (name, value) in previous {
        if let Some(value) = value {
            std::env::set_var(name, value);
        } else {
            std::env::remove_var(name);
        }
    }
}
