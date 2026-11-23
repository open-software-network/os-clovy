import AppKit
import Darwin
import AVFoundation
import CoreMedia
import ScreenCaptureKit
import UniformTypeIdentifiers

// TIMELINE: pauses use host clock boundaries, independent of sparse screen frames.
struct VideoTimeline {
    var first: CMTime?
    var pauseStarted: CMTime?
    var pausedDuration = CMTime.zero
    var pausedAtFirstFrame = CMTime.zero
    mutating func pause(at time: CMTime) { if pauseStarted == nil { pauseStarted = time } }
    mutating func resume(at time: CMTime) {
        if let start = pauseStarted { pausedDuration = CMTimeAdd(pausedDuration, CMTimeSubtract(time, start)); pauseStarted = nil }
    }
    mutating func outputTime(for time: CMTime) -> CMTime {
        if first == nil { first = time; pausedAtFirstFrame = pausedDuration }
        return CMTimeSubtract(CMTimeSubtract(time, first!), CMTimeSubtract(pausedDuration, pausedAtFirstFrame))
    }
}
struct VideoFrameCadence {
    var nextDeadline: CMTime?
    let interval = CMTime(value: 1, timescale: 5)
    func nextDelay(at time: CMTime) -> Double {
        guard let nextDeadline else { return 0 }
        return max(0, CMTimeSubtract(nextDeadline, time).seconds)
    }
    mutating func shouldWrite(at time: CMTime) -> Bool {
        guard nextDelay(at: time) == 0 else { return false }
        nextDeadline = CMTimeAdd(time, interval)
        return true
    }
}
// Commit only timestamps that advance the written stream at no more than five fps.
// A late control ledger may move adjusted time behind frames already committed.
struct VideoOutputGate {
    var lastSuccessful: CMTime?
    func canAppend(_ time: CMTime) -> Bool {
        guard let lastSuccessful else { return true }
        return CMTimeCompare(CMTimeSubtract(time, lastSuccessful), CMTime(value: 1, timescale: 5)) >= 0
    }
    mutating func didAppend(_ time: CMTime) { lastSuccessful = time }
}
struct VideoFrameFreshness {
    var paused = false
    var earliestSample: CMTime?
    mutating func transition(paused: Bool, at time: CMTime) {
        self.paused = paused
        // Controls carry floored host milliseconds: reject that whole millisecond on resume.
        earliestSample = CMTimeAdd(time, CMTime(value: 1, timescale: 1000))
    }
    func accepts(sampleTime: CMTime) -> Bool {
        guard !paused else { return false }
        guard let earliestSample else { return true }
        return sampleTime.isValid && sampleTime.isNumeric && CMTimeCompare(sampleTime, earliestSample) >= 0
    }
}
struct VideoResumeRefresh {
    var pendingSequence: UInt64?
    mutating func transition(paused: Bool, sequence: UInt64) { pendingSequence = paused ? nil : sequence }
    mutating func takeRequest(available: Bool) -> UInt64? {
        guard available else { return nil }
        defer { pendingSequence = nil }
        return pendingSequence
    }
    func accepts(sequence: UInt64, currentSequence: UInt64, paused: Bool, requestedAt: CMTime, cachedAt: CMTime?) -> Bool {
        guard sequence == currentSequence && !paused else { return false }
        return cachedAt == nil || CMTimeCompare(cachedAt!, requestedAt) < 0
    }
}
// END TIMELINE

struct VideoControlEvent: Decodable {
    let sequence: UInt64
    let paused: Bool
    let atHostMs: Int64
}
struct VideoControlState {
    var appliedSequence: UInt64 = 0
    var paused = false
    mutating func reconcile(_ events: [VideoControlEvent], timeline: inout VideoTimeline) {
        for event in events.sorted(by: { $0.sequence < $1.sequence }) where event.sequence > appliedSequence {
            let time = CMTime(value: event.atHostMs, timescale: 1000)
            if event.paused { timeline.pause(at: time) } else { timeline.resume(at: time) }
            paused = event.paused; appliedSequence = event.sequence
        }
    }
}

enum VideoParentObservation: Equatable {
    case present(startSec: UInt64, startUsec: UInt64)
    case terminated
    case unknown
}
enum VideoParentState { case alive, terminated, unknown }
struct VideoParentIdentity {
    let pid: Int32
    let initial: VideoParentObservation
    static func capture(pid: Int32) -> VideoParentIdentity { VideoParentIdentity(pid: pid, initial: observe(pid: pid)) }
    static func observe(pid: Int32) -> VideoParentObservation {
        guard pid > 1 else { return .unknown }
        var info = proc_bsdinfo()
        let size = Int32(MemoryLayout<proc_bsdinfo>.size)
        guard proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, size) == size else {
            return errno == ESRCH ? .terminated : .unknown
        }
        // Descriptor closure can precede SZOMB; an exiting process also cannot own capture.
        if info.pbi_status == UInt32(SZOMB) || (info.pbi_flags & UInt32(PROC_FLAG_INEXIT)) != 0 { return .terminated }
        return .present(startSec: info.pbi_start_tvsec, startUsec: info.pbi_start_tvusec)
    }
    func classify(_ observation: VideoParentObservation) -> VideoParentState {
        if initial == .terminated { return .terminated }
        switch observation {
        case .terminated: return .terminated
        case .unknown: return .unknown
        case .present:
            guard case .present = initial else { return .unknown }
            return observation == initial ? .alive : .terminated
        }
    }
    var state: VideoParentState { classify(Self.observe(pid: pid)) }
}
enum VideoControlReadState { case open, ended }
func videoControlEndFailure(parent: VideoParentState) -> String? {
    parent == .terminated ? nil : "capture_failed"
}

// Recorder mutates this only on the main actor, including errors received while stopping.
struct VideoShutdownFailure {
    private(set) var reason: String?
    mutating func record(_ incoming: String?) -> Bool {
        guard reason == nil, let incoming else { return false }
        reason = incoming
        return true
    }
    func resultReason(completed: Bool) -> String? {
        reason ?? (completed ? nil : "capture_failed")
    }
}

// A stopped SCK/AVAssetWriter callback can remain suspended after the host dies.
// This queue never waits for the actor, media APIs or filesystem before terminating.
final class VideoShutdownDeadline {
    private let work: DispatchWorkItem
    init(after seconds: Double = 4, reason: String?, publishFailure: (String) -> Void,
         onTimeout: @escaping @Sendable () -> Void = { Darwin._exit(0) }) {
        work = DispatchWorkItem(block: onTimeout)
        DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + seconds, execute: work)
        // Keep known failures durable before awaiting a possibly hung media shutdown.
        // A normal parent termination deliberately writes no failure marker.
        if let reason { publishFailure(reason) }
    }
    func cancel() { work.cancel() }
}

// Final status and media have already been flushed. Never run process exit hooks.
func finishVideoShutdown(_ deadline: VideoShutdownDeadline?) -> Never {
    deadline?.cancel()
    Darwin._exit(0)
}

// The host queues atomic fixed records before changing audio state. A full/broken channel fails video.
final class VideoControlPipe {
    let fd: Int32
    var pending = Data()
    var deferredEvents: [VideoControlEvent] = []
    private(set) var readState = VideoControlReadState.open
    init(path: String) throws {
        let opened = open(path, O_RDONLY | O_NONBLOCK | O_NOFOLLOW | O_CLOEXEC)
        guard opened >= 0 else { throw NSError(domain: "video-control", code: 1) }
        var info = stat()
        guard fstat(opened, &info) == 0 && (info.st_mode & S_IFMT) == S_IFIFO &&
            info.st_mode & 0o777 == 0o600 && info.st_uid == geteuid() else {
            close(opened); throw NSError(domain: "video-control", code: 2)
        }
        fd = opened
    }
    deinit { close(fd) }
    // nil means the bounded drain budget was exhausted: defer output until fully drained.
    func drain() throws -> [VideoControlEvent]? {
        if readState == .ended { return [] }
        var events = deferredEvents
        deferredEvents = []
        for _ in 0..<64 {
            var buffer = [UInt8](repeating: 0, count: 4096)
            let count = read(fd, &buffer, buffer.count)
            if count < 0 && (errno == EAGAIN || errno == EWOULDBLOCK) {
                guard pending.isEmpty else { throw NSError(domain: "video-control", code: 5) }
                return events
            }
            if count == 0 {
                guard pending.isEmpty else { throw NSError(domain: "video-control", code: 5) }
                readState = .ended
                return events
            }
            guard count > 0 else { throw NSError(domain: "video-control", code: 3) }
            pending.append(contentsOf: buffer.prefix(count))
            while pending.count >= 24 {
                let fields = pending.withUnsafeBytes { raw in
                    (UInt64(littleEndian: raw.loadUnaligned(fromByteOffset: 0, as: UInt64.self)),
                     UInt64(littleEndian: raw.loadUnaligned(fromByteOffset: 8, as: UInt64.self)),
                     UInt64(littleEndian: raw.loadUnaligned(fromByteOffset: 16, as: UInt64.self)))
                }
                guard fields.0 > 0 && fields.2 <= 1 && Int64(bitPattern: fields.1) >= 0 else { throw NSError(domain: "video-control", code: 4) }
                guard events.count < 4096 else { throw NSError(domain: "video-control", code: 6) }
                events.append(VideoControlEvent(sequence: fields.0, paused: fields.2 == 1, atHostMs: Int64(bitPattern: fields.1)))
                pending.removeFirst(24)
            }
        }
        deferredEvents = events
        return nil
    }
}

struct VideoCaptureLease {
    let url: URL
    let output: URL
    let status: URL
    let pid: URL
    let controls: URL
    var isActive: Bool { FileManager.default.fileExists(atPath: url.path) }
    func cleanupOutputs() {
        for owned in [output, status, pid, controls, controls.deletingPathExtension().appendingPathExtension("tmp"), controls.deletingPathExtension().appendingPathExtension("pipe"), output.deletingPathExtension().appendingPathExtension("timing.json"), output.deletingPathExtension().appendingPathExtension("capture.json"), output.deletingPathExtension().appendingPathExtension("capture.tmp")] { try? FileManager.default.removeItem(at: owned) }
    }
}

// Segment callbacks own one serialized file handle; they never create/reopen output after revocation.
final class VideoSegmentSink: NSObject, AVAssetWriterDelegate {
    let lease: VideoCaptureLease
    private let file: FileHandle
    private let lock = NSLock()
    private var failed = false
    private var closed = false
    var onFailure: (() -> Void)?
    init(lease: VideoCaptureLease) throws {
        guard lease.isActive else { throw NSError(domain: "video-segment", code: 1) }
        let fd = open(lease.output.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)
        guard fd >= 0 else { throw NSError(domain: "video-segment", code: 2) }
        guard lease.isActive else { close(fd); lease.cleanupOutputs(); throw NSError(domain: "video-segment", code: 3) }
        self.lease = lease; file = FileHandle(fileDescriptor: fd, closeOnDealloc: true)
    }
    func assetWriter(_ writer: AVAssetWriter, didOutputSegmentData data: Data, segmentType: AVAssetSegmentType, segmentReport: AVAssetSegmentReport?) {
        lock.lock(); defer { lock.unlock() }
        guard !failed, !closed else { return }
        guard lease.isActive else {
            failed = true; closed = true; try? file.close(); lease.cleanupOutputs(); onFailure?(); return
        }
        do {
            try file.write(contentsOf: data)
            try file.synchronize()
            if !lease.isActive { failed = true; closed = true; try? file.close(); lease.cleanupOutputs(); onFailure?() }
        } catch { failed = true; onFailure?() }
    }
    func finish() -> Bool {
        lock.lock(); defer { lock.unlock() }
        if !closed {
            do { try file.synchronize(); try file.close() } catch { failed = true }
            closed = true
        }
        return !failed
    }
}
func fragmentedVideoWriter(delegate: VideoSegmentSink) -> AVAssetWriter {
    let writer = AVAssetWriter(contentType: .mpeg4Movie)
    writer.outputFileTypeProfile = .mpeg4AppleHLS
    writer.preferredOutputSegmentInterval = CMTime(seconds: 10, preferredTimescale: 600)
    writer.initialSegmentStartTime = .zero
    writer.delegate = delegate
    return writer
}

// Keep this utility-title policy aligned with video_window_selection.rs.
// Recognize whole utility names after stripping known app labels, not subject words.
func isMeetingUtilityTitle(_ title: String) -> Bool {
    var core = title.split { !$0.isLetter && !$0.isNumber }.map(String.init)
    let labels = [["microsoft", "teams"], ["teams", "meeting"], ["zoom", "workplace"], ["zoom", "meeting"], ["teams"], ["zoom"]]
    while true {
        let previousCount = core.count
        for label in labels {
            if core.starts(with: label) {
                core.removeFirst(label.count)
                break
            }
            if core.suffix(label.count).elementsEqual(label) {
                core.removeLast(label.count)
                break
            }
        }
        if core.count == previousCount { break }
    }
    return ["settings", "preferences", "chat", "chats", "team chat", "meeting chat", "meeting settings", "meeting preferences", "calendar", "contacts", "sign in", "login", "configurações", "preferências", "conversas", "calendário", "contatos"].contains(core.joined(separator: " "))
}

func isMeetingWindow(bundle: String, title: String, microphoneOwned: Bool) -> Bool {
    let bundle = bundle.lowercased()
    let title = title.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    guard !title.isEmpty else { return false }
    func family(_ prefix: String) -> Bool { bundle == prefix || bundle.hasPrefix(prefix + ".") }
    let zoom = family("us.zoom.xos")
    let teams = family("com.microsoft.teams") || family("com.microsoft.teams2")
    if zoom || teams {
        guard !isMeetingUtilityTitle(title) else { return false }
        let indicators = zoom ? ["zoom meeting", "zoom webinar", "reunião do zoom", "reunião zoom", "webinar do zoom"] : ["teams meeting", "meeting | microsoft teams", "meeting - microsoft teams", "reunião do teams", "reunião do microsoft teams"]
        return indicators.contains { title.contains($0) } || (microphoneOwned && ["meeting", "webinar", "reunião"].contains(title))
    }
    let browser = ["company.thebrowser.browser", "com.google.chrome", "com.apple.safari", "org.mozilla.firefox", "com.microsoft.edgemac", "com.brave.browser", "com.operasoftware.opera"].contains { family($0) }
    return browser && isBrowserMeetingTitle(title)
}

func isBrowserMeetingTitle(_ title: String) -> Bool {
    var core = title
    for browser in ["google chrome", "chrome", "arc", "safari", "firefox", "microsoft edge", "brave", "opera"] {
        for separator in [" - ", " – ", " — "] {
            let suffix = separator + browser
            if core.hasSuffix(suffix) { core = String(core.dropLast(suffix.count)).trimmingCharacters(in: .whitespacesAndNewlines) }
        }
    }
    if [" - google search", " - bing", " - duckduckgo", " - search"].contains(where: { core.hasSuffix($0) }) { return false }
    if core.hasPrefix("meet - ") && !core.dropFirst(7).trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return true }
    if ["zoom meeting", "teams meeting", "meeting | microsoft teams", "reunião do teams", "reunião do microsoft teams"].contains(core) { return true }
    let url = core.hasPrefix("https://") ? String(core.dropFirst(8)) : core
    guard url.hasPrefix("meet.google.com/"), !url.contains(where: { $0.isWhitespace }) else { return false }
    let code = url.dropFirst(16).split(omittingEmptySubsequences: false, whereSeparator: { $0 == "?" || $0 == "#" }).first.map(String.init) ?? ""
    return code.utf8.count == 12 && code.utf8.enumerated().allSatisfy { index, byte in
        (index == 3 || index == 8) ? byte == 45 : (97...122).contains(byte)
    }
}

// ScreenCaptureKit windows are measured in points; stream/writer dimensions are pixels.
func videoCaptureSize(points: CGSize, pixelScale: CGFloat) -> (width: Int, height: Int)? {
    guard points.width.isFinite, points.height.isFinite, pixelScale.isFinite,
          points.width > 0, points.height > 0, pixelScale > 0 else { return nil }
    let pixels = CGSize(width: points.width * pixelScale, height: points.height * pixelScale)
    guard pixels.width.isFinite, pixels.height.isFinite else { return nil }
    let scale = min(1, 1920 / max(pixels.width, pixels.height))
    return (max(2, Int(pixels.width * scale) / 2 * 2),
            max(2, Int(pixels.height * scale) / 2 * 2))
}

struct VideoTimingCheckpoint: Codable {
    let schema: Int
    let recordingSessionId: String
    let partialPath: String
    let captureNonce: String
    let audioStartedHostMs: Int64
    let startedAtHostMs: Int64
    let pausedBeforeFirstMs: Int64
    let offsetMs: Int64
    let app: String
    let title: String
    let width: Int
    let height: Int
    init(session: String, partialPath: String, nonce: String, audioStart: Int64, first: Int64, pausedBeforeFirst: Int64,
         app: String, title: String, width: Int, height: Int) throws {
        let elapsed = first.subtractingReportingOverflow(audioStart)
        let offset = elapsed.partialValue.subtractingReportingOverflow(pausedBeforeFirst)
        guard !elapsed.overflow, !offset.overflow, offset.partialValue >= 0, pausedBeforeFirst >= 0,
            UUID(uuidString: nonce) != nil, !session.isEmpty else { throw NSError(domain: "video-timing", code: 1) }
        schema = 1; recordingSessionId = session; self.partialPath = partialPath; captureNonce = nonce
        audioStartedHostMs = audioStart; startedAtHostMs = first; pausedBeforeFirstMs = pausedBeforeFirst
        offsetMs = offset.partialValue; self.app = app; self.title = title; self.width = width; self.height = height
    }
    func writeDurably(to url: URL) throws {
        let bytes = try JSONEncoder().encode(self)
        try bytes.write(to: url, options: .atomic)
        let fd = open(url.path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
        guard fd >= 0 else { throw NSError(domain: "video-timing", code: 2) }
        defer { close(fd) }
        guard fsync(fd) == 0 else { throw NSError(domain: "video-timing", code: 3) }
        let directory = open(url.deletingLastPathComponent().path, O_RDONLY | O_DIRECTORY | O_CLOEXEC)
        guard directory >= 0 else { throw NSError(domain: "video-timing", code: 4) }
        defer { close(directory) }
        guard fsync(directory) == 0 else { throw NSError(domain: "video-timing", code: 5) }
    }
}

struct VideoProcessRecord: Codable {
    let pid: Int32
    let processStartSec: UInt64?
    let processStartUsec: UInt64?
    let captureNonce: String?
    static func current(nonce: String?) -> VideoProcessRecord {
        var info = proc_bsdinfo()
        let size = Int32(MemoryLayout<proc_bsdinfo>.size)
        let found = proc_pidinfo(getpid(), PROC_PIDTBSDINFO, 0, &info, size) == size
        return VideoProcessRecord(pid: getpid(), processStartSec: found ? info.pbi_start_tvsec : nil,
            processStartUsec: found ? info.pbi_start_tvusec : nil, captureNonce: nonce)
    }
}

// The main actor owns writer, timing, pause and stop state.
@MainActor
final class Recorder: NSObject, SCStreamOutput, SCStreamDelegate {
    let queue = DispatchQueue.main
    let output: URL
    let statusURL: URL
    let pidURL: URL
    let processRecord: VideoProcessRecord
    let recordingSessionId: String
    let audioStartedHostMs: Int64
    let lease: VideoCaptureLease
    var leaseRevoked = false
    var shutdownDeadline: VideoShutdownDeadline?
    var shutdownFailure = VideoShutdownFailure()
    let windowID: UInt32
    var stream: SCStream?
    var writer: AVAssetWriter?
    var segmentSink: VideoSegmentSink?
    var input: AVAssetWriterInput?
    var app = ""
    var title = ""
    var width = 0
    var height = 0
    var controlState = VideoControlState()
    var controlPipe: VideoControlPipe?
    var freshness = VideoFrameFreshness()
    var resumeRefresh = VideoResumeRefresh()
    var captureFilter: SCContentFilter?
    var captureConfiguration: SCStreamConfiguration?
    var paused: Bool { controlState.paused }
    var stopping = false
    var timeline = VideoTimeline()
    var cadence = VideoFrameCadence()
    var outputGate = VideoOutputGate()
    var lastOutputTime = CMTime.zero
    var startedAtHostMs: Int64?
    var audioOffsetMs: Int64?
    var signals: [DispatchSourceSignal] = []
    var parentWatch: DispatchSourceTimer?
    var parentIdentity: VideoParentIdentity?
    var frameTimer: DispatchSourceTimer?
    var latestSample: CMSampleBuffer?

    init(windowID: UInt32, output: URL, status: URL, pid: URL, lease: URL, controls: URL, session: String, nonce: String, audioStart: Int64) {
        recordingSessionId = session; audioStartedHostMs = audioStart; processRecord = VideoProcessRecord.current(nonce: nonce)
        self.windowID = windowID; self.output = output; statusURL = status; pidURL = pid; self.lease = VideoCaptureLease(url: lease, output: output, status: status, pid: pid, controls: controls)
    }
    var leaseAvailable: Bool { !leaseRevoked && lease.isActive }
    func cleanupRevokedFiles() {
        leaseRevoked = true
        lease.cleanupOutputs()
    }
    func status(_ event: String, reason: String? = nil) {
        guard leaseAvailable else { cleanupRevokedFiles(); return }
        var value: [String: Any] = ["event": event, "pid": ProcessInfo.processInfo.processIdentifier,
            "app": app, "title": title, "width": width, "height": height,
            "durationMs": Int64(max(0, lastOutputTime.seconds) * 1000)]
        if let startedAtHostMs { value["startedAtHostMs"] = startedAtHostMs }
        if let audioOffsetMs { value["audioOffsetMs"] = audioOffsetMs }
        if let nonce = processRecord.captureNonce { value["captureNonce"] = nonce }
        if let sec = processRecord.processStartSec { value["processStartSec"] = sec }
        if let usec = processRecord.processStartUsec { value["processStartUsec"] = usec }
        if let reason { value["failureReason"] = reason }
        if let bytes = try? JSONSerialization.data(withJSONObject: value) {
            guard leaseAvailable else { cleanupRevokedFiles(); return }
            try? bytes.write(to: statusURL, options: .atomic)
            if !leaseAvailable { cleanupRevokedFiles() }
        }
    }
    @discardableResult
    func reconcileControls(parentEnded: Bool = false) -> Bool {
        guard !stopping else { return false }
        guard leaseAvailable else { stop(revoked: true); return false }
        guard let controlPipe else { return false }
        do {
            guard let events = try controlPipe.drain() else { return false }
            for event in events {
                guard event.sequence == controlState.appliedSequence + 1 else { stop(reason: "capture_failed"); return false }
                latestSample = nil
                freshness.transition(paused: event.paused, at: CMTime(value: event.atHostMs, timescale: 1000))
                resumeRefresh.transition(paused: event.paused, sequence: event.sequence)
                controlState.reconcile([event], timeline: &timeline)
            }
            if controlPipe.readState == .ended {
                // Clean owner termination preserves fragments; a live/unknown owner losing control is failure.
                stop(reason: videoControlEndFailure(parent: parentEnded ? .terminated : (parentIdentity?.state ?? .unknown)))
                return false
            }
            requestFreshFrame()
            return true
        } catch { stop(reason: "capture_failed"); return false }
    }
    func requestFreshFrame() {
        guard !stopping, !paused, let filter = captureFilter, let configuration = captureConfiguration,
            let sequence = resumeRefresh.takeRequest(available: stream != nil) else { return }
        guard #available(macOS 14.0, *) else { stop(reason: "capture_failed"); return }
        Task { @MainActor in
            // The host record uses floored milliseconds. Start beyond its conservative fence.
            try? await Task.sleep(nanoseconds: 1_000_000)
            guard self.reconcileControls(), !self.stopping, self.leaseAvailable,
                self.controlState.appliedSequence == sequence, !self.paused else { return }
            let requestedAt = CMClockGetTime(CMClockGetHostTimeClock())
            do {
                let sample = try await SCScreenshotManager.captureSampleBuffer(contentFilter: filter, configuration: configuration)
                guard self.reconcileControls(), !self.stopping, self.leaseAvailable,
                    self.resumeRefresh.accepts(sequence: sequence, currentSequence: self.controlState.appliedSequence,
                        paused: self.paused, requestedAt: requestedAt,
                        cachedAt: self.latestSample.map { CMSampleBufferGetPresentationTimeStamp($0) }) else { return }
                guard sample.isValid, CMSampleBufferGetImageBuffer(sample) != nil,
                    self.freshness.accepts(sampleTime: requestedAt) else { self.stop(reason: "capture_failed"); return }
                var timing = CMSampleTimingInfo(duration: CMTime(value: 1, timescale: 5), presentationTimeStamp: requestedAt, decodeTimeStamp: .invalid)
                var fresh: CMSampleBuffer?
                guard CMSampleBufferCreateCopyWithNewTiming(allocator: kCFAllocatorDefault, sampleBuffer: sample,
                    sampleTimingEntryCount: 1, sampleTimingArray: &timing, sampleBufferOut: &fresh) == noErr,
                    let fresh else { self.stop(reason: "capture_failed"); return }
                self.latestSample = fresh
            } catch {
                guard !self.stopping, self.leaseAvailable, self.controlState.appliedSequence == sequence, !self.paused else { return }
                if self.latestSample == nil { self.stop(reason: "capture_failed") }
            }
        }
    }
    func installSignals() {
        for number in [SIGUSR1, SIGUSR2, SIGTERM, SIGINT] {
            signal(number, SIG_IGN)
            let source = DispatchSource.makeSignalSource(signal: number, queue: queue)
            source.setEventHandler { [weak self] in
                MainActor.assumeIsolated {
                    guard let self else { return }
                    self.reconcileControls()
                    if number != SIGUSR1 && number != SIGUSR2 { self.stop() }
                }
            }
            source.resume(); signals.append(source)
        }
    }
    func start() async {
        parentIdentity = argument("--parent").flatMap(Int32.init).map { VideoParentIdentity.capture(pid: $0) }
        installSignals()
        // Installed before any PID/status/output creation and kept alive through permission waits.
        let watch = DispatchSource.makeTimerSource(queue: queue)
        watch.schedule(deadline: .now(), repeating: .milliseconds(200))
        watch.setEventHandler { [weak self] in MainActor.assumeIsolated {
            guard let self else { return }
            if !self.leaseAvailable { self.stop(revoked: true) }
            else if self.parentIdentity?.state == .terminated {
                // Preserve final queued pause boundaries, while malformed control data still fails video.
                self.reconcileControls(parentEnded: true)
                self.stop()
            }
            else { self.reconcileControls() }
        } }
        watch.resume(); parentWatch = watch
        guard leaseAvailable else { cleanupRevokedFiles(); exit(0) }
        do { controlPipe = try VideoControlPipe(path: lease.controls.deletingPathExtension().appendingPathExtension("pipe").path) }
        catch { status("failed", reason: "capture_failed"); exit(0) }
        guard reconcileControls() else { stop(reason: "capture_failed"); return }
        if let bytes = try? JSONEncoder().encode(processRecord) { try? bytes.write(to: pidURL, options: .atomic) }
        guard leaseAvailable else { cleanupRevokedFiles(); exit(0) }
        // Permission UI must not block the actor that receives stop and parent-death signals.
        let granted = await Task.detached { CGPreflightScreenCaptureAccess() || CGRequestScreenCaptureAccess() }.value
        guard !stopping else { return }
        guard leaseAvailable else { stop(revoked: true); return }
        guard granted else {
            status("failed", reason: "permission_denied"); exit(0)
        }
        do {
            let content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: true)
            guard !stopping else { return }
            guard leaseAvailable else { stop(revoked: true); return }
            guard let window = content.windows.first(where: { $0.windowID == windowID }) else {
                status("failed", reason: "capture_failed"); exit(0)
            }
            let bundle = (window.owningApplication?.bundleIdentifier ?? "").lowercased()
            let name = (window.title ?? "").lowercased()
            guard isMeetingWindow(bundle: bundle, title: name, microphoneOwned: argument("--microphone-owned") == "true") else {
                status("failed", reason: "capture_failed"); exit(0)
            }
            app = window.owningApplication?.applicationName ?? ""
            title = window.title ?? ""
            let filter = SCContentFilter(desktopIndependentWindow: window)
            let pixelScale = CGFloat(SCShareableContent.info(for: filter).pointPixelScale)
            guard let size = videoCaptureSize(points: window.frame.size, pixelScale: pixelScale) else {
                throw NSError(domain: "video", code: 3)
            }
            width = size.width; height = size.height
            let configuration = SCStreamConfiguration()
            configuration.width = width; configuration.height = height
            configuration.minimumFrameInterval = CMTime(value: 1, timescale: 5)
            configuration.capturesAudio = false
            configuration.showsCursor = false
            configuration.queueDepth = 3
            let sink = try VideoSegmentSink(lease: lease)
            sink.onFailure = { [weak self] in Task { @MainActor in self?.stop(reason: "capture_failed") } }
            segmentSink = sink
            let writer = fragmentedVideoWriter(delegate: sink)
            writer.shouldOptimizeForNetworkUse = false
            let input = AVAssetWriterInput(mediaType: .video, outputSettings: [
                AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: width, AVVideoHeightKey: height,
                AVVideoCompressionPropertiesKey: [AVVideoAverageBitRateKey: 1_000_000,
                    AVVideoMaxKeyFrameIntervalKey: 25, AVVideoExpectedSourceFrameRateKey: 5]])
            input.expectsMediaDataInRealTime = true
            guard writer.canAdd(input) else { throw NSError(domain: "video", code: 1) }
            writer.add(input); self.writer = writer; self.input = input
            guard writer.startWriting() else { throw writer.error ?? NSError(domain: "video", code: 2) }
            writer.startSession(atSourceTime: .zero)
            captureFilter = filter; captureConfiguration = configuration
            let stream = SCStream(filter: filter, configuration: configuration, delegate: self)
            self.stream = stream
            try stream.addStreamOutput(self, type: .screen, sampleHandlerQueue: queue)
            status("starting")
            guard leaseAvailable else { stop(revoked: true); return }
            // Repeat the last complete screen frame at 5 fps. Static slides must still
            // produce 10-second fragments that survive an unexpected termination.
            let timer = DispatchSource.makeTimerSource(queue: .main)
            timer.setEventHandler { [weak self] in MainActor.assumeIsolated { self?.writeFrame() } }
            frameTimer = timer; scheduleFrame(); timer.resume()
            try await stream.startCapture()
            if stopping { try? await stream.stopCapture() } else { requestFreshFrame() }
        } catch {
            if !stopping {
                let ns = error as NSError
                stop(reason: ns.domain == SCStreamErrorDomain && ns.code == SCStreamError.Code.userDeclined.rawValue ? "permission_denied" : "capture_failed")
            }
        }
    }
    nonisolated func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        // Registered with the main queue; consume synchronously while the sample is retained.
        MainActor.assumeIsolated { consume(sampleBuffer, type: type) }
    }
    private func consume(_ sampleBuffer: CMSampleBuffer, type: SCStreamOutputType) {
        guard type == .screen, !stopping, reconcileControls(), !paused else { return }
        if let attachments = CMSampleBufferGetSampleAttachmentsArray(sampleBuffer, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
           let raw = attachments.first?[.status] as? Int {
            if raw == SCFrameStatus.stopped.rawValue { stop(); return }
            if raw != SCFrameStatus.complete.rawValue { return }
        }
        guard sampleBuffer.isValid, freshness.accepts(sampleTime: CMSampleBufferGetPresentationTimeStamp(sampleBuffer)) else { return }
        if let latestSample, CMTimeCompare(CMSampleBufferGetPresentationTimeStamp(sampleBuffer), CMSampleBufferGetPresentationTimeStamp(latestSample)) < 0 { return }
        latestSample = sampleBuffer
    }
    private func scheduleFrame(after minimumDelay: Double = 0) {
        guard !stopping, let timer = frameTimer else { return }
        let delay = max(0.0001, minimumDelay, cadence.nextDelay(at: CMClockGetTime(CMClockGetHostTimeClock())))
        // One-shot rearming defers an early wakeup to its deadline instead of dropping a whole tick.
        timer.schedule(deadline: .now() + delay, leeway: .microseconds(50))
    }
    private func writeFrame() {
        guard reconcileControls() else { scheduleFrame(after: 0.2); return }
        guard !stopping, !paused, let sampleBuffer = latestSample, let input,
              input.isReadyForMoreMediaData else { scheduleFrame(after: 0.2); return }
        let sequence = controlState.appliedSequence
        let timestamp = CMClockGetTime(CMClockGetHostTimeClock())
        guard cadence.shouldWrite(at: timestamp) else { scheduleFrame(); return }
        var candidateTimeline = timeline
        let adjusted = candidateTimeline.outputTime(for: timestamp)
        guard outputGate.canAppend(adjusted) else { scheduleFrame(); return }
        var timing = CMSampleTimingInfo(duration: CMTime(value: 1, timescale: 5), presentationTimeStamp: adjusted, decodeTimeStamp: .invalid)
        var copy: CMSampleBuffer?
        guard CMSampleBufferCreateCopyWithNewTiming(allocator: kCFAllocatorDefault, sampleBuffer: sampleBuffer,
            sampleTimingEntryCount: 1, sampleTimingArray: &timing, sampleBufferOut: &copy) == noErr,
            let copy else { scheduleFrame(); return }
        var firstCheckpoint: VideoTimingCheckpoint?
        if timeline.first == nil {
            do {
                guard let nonce = processRecord.captureNonce else { throw NSError(domain: "video-timing", code: 6) }
                let checkpoint = try VideoTimingCheckpoint(session: recordingSessionId, partialPath: output.path,
                    nonce: nonce, audioStart: audioStartedHostMs, first: Int64(timestamp.seconds * 1000),
                    pausedBeforeFirst: Int64(candidateTimeline.pausedAtFirstFrame.seconds * 1000),
                    app: app, title: title, width: width, height: height)
                guard leaseAvailable else { stop(revoked: true); return }
                try checkpoint.writeDurably(to: output.deletingPathExtension().appendingPathExtension("timing.json"))
                guard leaseAvailable else { stop(revoked: true); return }
                firstCheckpoint = checkpoint
            } catch { stop(reason: "capture_failed"); return }
        }
        // Drain again immediately before append; a pause never waits for the worker/JSON ledger.
        guard reconcileControls(), controlState.appliedSequence == sequence, !paused, freshness.accepts(sampleTime: CMSampleBufferGetPresentationTimeStamp(sampleBuffer)) else { scheduleFrame(); return }
        if input.append(copy) {
            timeline = candidateTimeline
            if let firstCheckpoint { startedAtHostMs = firstCheckpoint.startedAtHostMs; audioOffsetMs = firstCheckpoint.offsetMs }
            outputGate.didAppend(adjusted)
            lastOutputTime = adjusted
            status("recording")
        } else { stop(reason: "capture_failed") }
        scheduleFrame()
    }
    nonisolated func stream(_ stream: SCStream, didStopWithError error: Error) {
        // Connection loss to a closed window preserves the valid partial video.
        let ns = error as NSError
        let windowClosed = ns.domain == SCStreamErrorDomain && [
            SCStreamError.Code.failedApplicationConnectionInvalid.rawValue,
            SCStreamError.Code.failedApplicationConnectionInterrupted.rawValue,
            SCStreamError.Code.failedNoMatchingApplicationContext.rawValue
        ].contains(ns.code)
        let denied = ns.domain == SCStreamErrorDomain && ns.code == SCStreamError.Code.userDeclined.rawValue
        Task { @MainActor in
            self.stop(reason: windowClosed && self.timeline.first != nil ? nil : (denied ? "permission_denied" : "capture_failed"))
        }
    }
    func stop(reason: String? = nil, revoked: Bool = false) {
        if shutdownDeadline == nil {
            shutdownDeadline = VideoShutdownDeadline(reason: nil, publishFailure: { _ in })
        }
        // A normal stop may already be awaiting SCK or the writer when a real error
        // arrives. Publish it before the stopping guard and keep it through finalization.
        if shutdownFailure.record(reason) { status("failed", reason: shutdownFailure.reason) }
        if revoked || !leaseAvailable {
            // Deleted sessions need no graceful media flush. Exit even if SCK is stuck stopping.
            leaseRevoked = true; stopping = true
            frameTimer?.cancel(); frameTimer = nil
            writer?.cancelWriting()
            cleanupRevokedFiles()
            exit(0)
        }
        guard !stopping else { return }; stopping = true
        frameTimer?.cancel(); frameTimer = nil
        let endHostTime = timeline.pauseStarted ?? CMClockGetTime(CMClockGetHostTimeClock())
        Task { @MainActor in
            try? await stream?.stopCapture()
            if !leaseAvailable {
                writer?.cancelWriting(); cleanupRevokedFiles(); exit(0)
            }
            guard let writer, let input else {
                status("failed", reason: shutdownFailure.resultReason(completed: false)); exit(0)
            }
            if timeline.first != nil {
                let endTime = timeline.outputTime(for: endHostTime)
                if CMTimeCompare(endTime, lastOutputTime) > 0 {
                    writer.endSession(atSourceTime: endTime)
                    lastOutputTime = endTime
                }
            }
            input.markAsFinished()
            writer.finishWriting {
                Task { @MainActor in
                    guard self.leaseAvailable else { self.cleanupRevokedFiles(); exit(0) }
                    let completed = self.writer?.status == .completed && self.segmentSink?.finish() == true
                    self.status(completed ? "stopped" : "failed",
                        reason: self.shutdownFailure.resultReason(completed: completed))
                    finishVideoShutdown(self.shutdownDeadline)
                }
            }
        }
    }
}

func argument(_ name: String) -> String? {
    let args = CommandLine.arguments
    guard let index = args.firstIndex(of: name), index + 1 < args.count else { return nil }
    return args[index + 1]
}
guard let id = argument("--window").flatMap(UInt32.init),
      let output = argument("--output"), let status = argument("--status"), let pid = argument("--pid"), let lease = argument("--lease"), let controls = argument("--controls"), let session = argument("--recording-session"),
      let nonce = argument("--capture-nonce"), UUID(uuidString: nonce) != nil,
      let audioStart = argument("--audio-started-host-ms").flatMap(Int64.init) else { exit(2) }
let recorder = MainActor.assumeIsolated { Recorder(windowID: id, output: URL(fileURLWithPath: output), status: URL(fileURLWithPath: status), pid: URL(fileURLWithPath: pid), lease: URL(fileURLWithPath: lease), controls: URL(fileURLWithPath: controls), session: session, nonce: nonce, audioStart: audioStart) }
Task { await recorder.start() }
RunLoop.main.run()
