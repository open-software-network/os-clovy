import Foundation
import CoreMedia

@main
struct VideoLeaseTests {
    static func main() throws {
        if CommandLine.arguments.contains("--hung-shutdown") {
            // Forced shutdown must bypass even a permanently blocked atexit hook.
            atexit { DispatchSemaphore(value: 0).wait() }
            let status = URL(fileURLWithPath: CommandLine.arguments[2])
            let mode = CommandLine.arguments[3]
            let reason = mode == "failure" ? "capture_failed" : nil
            var failure = VideoShutdownFailure()
            _ = failure.record(reason)
            let deadline = VideoShutdownDeadline(after: 0.15, reason: reason) { reason in
                try! Data(("failed:" + reason).utf8).write(to: status, options: .atomic)
            }
            if mode.hasPrefix("late_") {
                assert(failure.record("capture_failed"))
                assert(!failure.record("permission_denied"))
                try Data(("failed:" + failure.reason!).utf8).write(to: status, options: .atomic)
            }
            if mode == "completed" || mode == "late_completed" {
                let reason = failure.resultReason(completed: true) ?? "none"
                try Data(("stopped:" + reason).utf8).write(to: status, options: .atomic)
                finishVideoShutdown(deadline)
            }
            // Models a permanently suspended stopCapture/finishWriting completion.
            withExtendedLifetime(deadline) { DispatchSemaphore(value: 0).wait() }
            return
        }
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("clovy-video-lease-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let lease = VideoCaptureLease(url: dir.appendingPathComponent("lease"), output: dir.appendingPathComponent("partial.mp4"), status: dir.appendingPathComponent("status.json"), pid: dir.appendingPathComponent("pid"), controls: dir.appendingPathComponent("control.json"))
        try Data("active".utf8).write(to: lease.url)
        try Data("video".utf8).write(to: lease.output)
        // Normal finish keeps its lease until the writer is settled, preserving video.
        assert(lease.isActive)
        if !lease.isActive { lease.cleanupOutputs() }
        assert(FileManager.default.fileExists(atPath: lease.output.path))
        // Model a helper that checked the lease, stalled, then created files after cancellation.
        try FileManager.default.removeItem(at: lease.url)
        let ownedMetadata = [lease.output, lease.status, lease.pid, lease.controls, lease.output.deletingPathExtension().appendingPathExtension("timing.json"), lease.output.deletingPathExtension().appendingPathExtension("capture.json"), lease.output.deletingPathExtension().appendingPathExtension("capture.tmp")]
        for url in ownedMetadata { try Data("late".utf8).write(to: url) }
        if !lease.isActive { lease.cleanupOutputs() }
        assert(ownedMetadata.allSatisfy { !FileManager.default.fileExists(atPath: $0.path) })
        func time(_ n: Double) -> CMTime { CMTime(seconds:n, preferredTimescale:1000) }
        var timeline = VideoTimeline()
        assert(timeline.outputTime(for:time(0)).seconds == 0)
        timeline.pause(at:time(10)); timeline.resume(at:time(15))
        assert(timeline.outputTime(for:time(20)).seconds == 15)
        var startup = VideoTimeline()
        startup.pause(at:time(2)); startup.resume(at:time(5))
        assert(startup.outputTime(for:time(7)).seconds == 0)
        assert(startup.outputTime(for:time(9)).seconds == 2)
        for (width, height, density, expectedWidth, expectedHeight) in [
            (1440.0, 900.0, 2.0, 1920, 1200), (1440.0, 900.0, 1.0, 1440, 900),
            (800.0, 600.0, 2.0, 1600, 1200), (900.0, 1440.0, 2.0, 1200, 1920),
            (101.0, 51.0, 1.5, 150, 76)
        ] {
            let size = videoCaptureSize(points: CGSize(width: width, height: height), pixelScale: density)!
            assert(size.width == expectedWidth && size.height == expectedHeight)
        }
        for density in [0.0, -1.0, Double.nan, Double.infinity] {
            assert(videoCaptureSize(points: CGSize(width: 100, height: 100), pixelScale: density) == nil)
        }
        assert(videoCaptureSize(points: CGSize(width: Double.infinity, height: 100), pixelScale: 2) == nil)
        assert(videoCaptureSize(points: CGSize(width: Double.greatestFiniteMagnitude, height: 100), pixelScale: 2) == nil)
        var cadence = VideoFrameCadence()
        assert([0.0,0.1,0.2,1.2,1.21,1.4].map { cadence.shouldWrite(at:time($0)) } == [true,false,true,true,false,true])
        for (bundle,title) in [("us.zoom.xos","Settings"),("us.zoom.xos","Zoom Meeting Settings"),("us.zoom.xos","Zoom Workplace"),("com.microsoft.teams2","Chat | Microsoft Teams"),("com.microsoft.teams2","Calendar")] {
            assert(!isMeetingWindow(bundle:bundle,title:title,microphoneOwned:true))
        }
        for title in ["Meet - planning", "Meet - abc-defg-hij - Google Chrome", "https://meet.google.com/abc-defg-hij", "Zoom Meeting", "Teams meeting", "Meeting | Microsoft Teams"] {
            assert(isMeetingWindow(bundle: "com.google.Chrome", title: title, microphoneOwned: false), title)
        }
        for title in ["Google Meet", "Google Meet - Google Search", "Google Meet API documentation", "How to use Google Meet", "Zoom meeting guide", "Teams meeting help", "Meet - abc-defg-hij - Google Search", "meet.google.com documentation", "https://meet.google.com/landing", "https://meet.google.com/?abc-defg-hij", "https://meet.google.com/#abc-defg-hij", "https://meet.google.com/abc-defg-hij?guide text"] {
            assert(!isMeetingWindow(bundle: "company.thebrowser.Browser", title: title, microphoneOwned: true), title)
        }
        for browser in ["company.thebrowser.browser", "com.google.chrome", "com.apple.safari", "org.mozilla.firefox", "com.microsoft.edgemac", "com.brave.browser", "com.operasoftware.opera"] {
            for bundle in [browser.uppercased(), browser + ".helper"] {
                assert(isMeetingWindow(bundle: bundle, title: "Meet - planning", microphoneOwned: false))
                assert(!isMeetingWindow(bundle: bundle, title: "Email - Inbox", microphoneOwned: false))
            }
            assert(!isMeetingWindow(bundle: browser + "archive", title: "Google Meet notes", microphoneOwned: false))
        }
        for bundle in ["com.example.knowledgebase", "com.google.chromeremotedesktop", "com.example.firefox"] {
            assert(!isMeetingWindow(bundle: bundle, title: "Google Meet notes", microphoneOwned: false))
        }
        for bundle in ["company.thebrowser.Browser", "company.thebrowser.Browser.helper"] {
            assert(isMeetingWindow(bundle: bundle, title: "Meet - planning", microphoneOwned: false))
            assert(!isMeetingWindow(bundle: bundle, title: "Email - Inbox", microphoneOwned: false))
        }
        for bundle in ["com.apple.searchapp", "com.apple.archiveutility", "company.thebrowser.BrowserArchive", "com.example.arc"] {
            assert(!isMeetingWindow(bundle: bundle, title: "Meet - planning", microphoneOwned: false))
        }
        assert(isMeetingWindow(bundle:"us.zoom.xos",title:"Zoom Meeting",microphoneOwned:false))
        assert(!isMeetingWindow(bundle:"us.zoom.xos",title:"Meeting",microphoneOwned:false))
        assert(isMeetingWindow(bundle:"us.zoom.xos",title:"Meeting",microphoneOwned:true))
        for (bundle, title) in [("com.microsoft.teams2", "Calendar planning meeting | Microsoft Teams"), ("com.microsoft.teams2", "Chat roadmap meeting | Microsoft Teams"), ("us.zoom.xos", "Calendar planning - Zoom Meeting"), ("us.zoom.xos", "Chat roadmap - Zoom Meeting")] {
            assert(isMeetingWindow(bundle: bundle, title: title, microphoneOwned: false), title)
        }
        for title in ["Meeting chat | Microsoft Teams", "Settings | Teams meeting", "Calendar | Teams meeting", "Zoom Meeting Settings", "Preferences - Zoom Meeting", "Meeting chat - Zoom Meeting", "Zoom Team Chat"] {
            for bundle in ["us.zoom.xos", "com.microsoft.teams2"] {
                assert(!isMeetingWindow(bundle: bundle, title: title, microphoneOwned: true), title)
            }
        }
        assert(!isMeetingWindow(bundle: "com.microsoft.teams2", title: "Calendar planning meeting", microphoneOwned: true))
        var jitterCadence = VideoFrameCadence()
        assert(jitterCadence.shouldWrite(at: time(0)))
        assert(!jitterCadence.shouldWrite(at: time(0.199)))
        assert(abs(jitterCadence.nextDelay(at: time(0.199)) - 0.001) < 0.000001)
        assert(jitterCadence.shouldWrite(at: time(0.200)))
        assert(jitterCadence.shouldWrite(at: time(0.405)))
        assert(!jitterCadence.shouldWrite(at: time(0.599)))
        assert(abs(jitterCadence.nextDelay(at: time(0.599)) - 0.006) < 0.000001)
        assert(jitterCadence.shouldWrite(at: time(0.605)))
        assert(!jitterCadence.shouldWrite(at: time(0.606)))
        assert(abs(jitterCadence.nextDelay(at: time(0.606)) - 0.199) < 0.000001)
        var rearmed = VideoFrameCadence()
        var delivered: [CMTime] = []
        for arrival in [0.0,0.199,0.405,0.599,0.800] {
            let callback = time(arrival)
            let deadline = CMTimeAdd(callback, CMTime(seconds: rearmed.nextDelay(at: callback), preferredTimescale: 1000))
            assert(rearmed.shouldWrite(at: deadline))
            delivered.append(deadline)
        }
        assert(delivered.count == 5, "early jitter must rearm rather than lose a scheduled frame")
        for pair in zip(delivered, delivered.dropFirst()) {
            assert(CMTimeCompare(CMTimeSubtract(pair.1, pair.0), CMTime(value: 1, timescale: 5)) >= 0)
        }
        let pipeURL = dir.appendingPathComponent("control.pipe")
        assert(mkfifo(pipeURL.path, 0o600) == 0)
        let writerFD = open(pipeURL.path, O_RDWR | O_NONBLOCK | O_NOFOLLOW)
        assert(writerFD >= 0)
        defer { close(writerFD) }
        let controlPipe = try VideoControlPipe(path: pipeURL.path)
        func record(_ sequence: UInt64, _ at: UInt64, _ paused: UInt64) -> Data {
            var bytes = Data()
            for var value in [sequence.littleEndian, at.littleEndian, paused.littleEndian] { withUnsafeBytes(of: &value) { bytes.append(contentsOf: $0) } }
            return bytes
        }
        let queued = record(1,1000,1) + record(2,3000,0)
        assert(queued.withUnsafeBytes { write(writerFD,$0.baseAddress,queued.count) } == queued.count)
        let replay = try controlPipe.drain()!
        assert(replay.map(\.sequence) == [1,2] && replay.map(\.paused) == [true,false])
        let emptied = try controlPipe.drain()
        assert(emptied!.isEmpty)
        let malformed = record(3,4000,2)
        assert(malformed.withUnsafeBytes { write(writerFD,$0.baseAddress,malformed.count) } == malformed.count)
        do { _ = try controlPipe.drain(); assertionFailure("invalid control must fail video") } catch {}
        let eofURL = dir.appendingPathComponent("eof.pipe")
        assert(mkfifo(eofURL.path, 0o600) == 0)
        let eofWriter = open(eofURL.path, O_RDWR | O_NONBLOCK | O_NOFOLLOW | O_CLOEXEC)
        assert(eofWriter >= 0)
        let eofPipe = try VideoControlPipe(path: eofURL.path)
        assert(queued.withUnsafeBytes { write(eofWriter, $0.baseAddress, queued.count) } == queued.count)
        close(eofWriter)
        let endedEvents = try? eofPipe.drain()
        if endedEvents == nil { print("FAIL: complete control records followed by writer EOF must remain recoverable"); exit(1) }
        assert(endedEvents?.map(\.sequence) == [1,2], "clean EOF must preserve the final queued pause/resume boundaries")
        assert(eofPipe.readState == .ended)
        var endedTimeline = VideoTimeline()
        var endedControls = VideoControlState()
        endedControls.reconcile(endedEvents!, timeline: &endedTimeline)
        assert(endedControls.appliedSequence == 2 && endedTimeline.pausedDuration.seconds == 2)
        let parent = VideoParentIdentity(pid: 42, initial: .present(startSec: 10, startUsec: 20))
        assert(parent.classify(.present(startSec: 10, startUsec: 20)) == .alive)
        assert(parent.classify(.terminated) == .terminated)
        assert(parent.classify(.present(startSec: 11, startUsec: 20)) == .terminated, "PID reuse means the original owner ended")
        assert(parent.classify(.unknown) == .unknown)
        assert(VideoParentIdentity(pid: 42, initial: .unknown).classify(.present(startSec: 10, startUsec: 20)) == .unknown)
        assert(VideoParentIdentity(pid: 42, initial: .terminated).classify(.present(startSec: 10, startUsec: 20)) == .terminated)
        assert(videoControlEndFailure(parent: .terminated) == nil)
        assert(videoControlEndFailure(parent: .alive) == "capture_failed")
        assert(videoControlEndFailure(parent: .unknown) == "capture_failed")
        assert(VideoParentIdentity.capture(pid: getpid()).state == .alive)
        let shortURL = dir.appendingPathComponent("short-eof.pipe")
        assert(mkfifo(shortURL.path, 0o600) == 0)
        let shortWriter = open(shortURL.path, O_RDWR | O_NONBLOCK | O_NOFOLLOW | O_CLOEXEC)
        assert(shortWriter >= 0)
        let shortPipe = try VideoControlPipe(path: shortURL.path)
        let shortRecord = Data([1,2,3])
        assert(shortRecord.withUnsafeBytes { write(shortWriter, $0.baseAddress, shortRecord.count) } == shortRecord.count)
        close(shortWriter)
        do { _ = try shortPipe.drain(); assertionFailure("truncated EOF remains a genuine capture failure") } catch {}
        assert(shortPipe.readState == .open)
        let ownerURL = dir.appendingPathComponent("owner-eof.pipe")
        assert(mkfifo(ownerURL.path, 0o600) == 0)
        let ownerWriter = open(ownerURL.path, O_RDWR | O_NONBLOCK | O_NOFOLLOW | O_CLOEXEC)
        assert(ownerWriter >= 0)
        let ownerHandle = FileHandle(fileDescriptor: ownerWriter, closeOnDealloc: true)
        let ownerPipe = try VideoControlPipe(path: ownerURL.path)
        let fakeOwner = Process()
        fakeOwner.executableURL = URL(fileURLWithPath: "/usr/bin/tail")
        fakeOwner.arguments = ["-f", "/dev/null"]
        fakeOwner.standardInput = ownerHandle
        fakeOwner.standardOutput = FileHandle.nullDevice
        fakeOwner.standardError = FileHandle.nullDevice
        try fakeOwner.run()
        defer { if fakeOwner.isRunning { fakeOwner.terminate(); fakeOwner.waitUntilExit() } }
        let realOwner = VideoParentIdentity.capture(pid: fakeOwner.processIdentifier)
        try ownerHandle.close()
        let liveOwnerState = realOwner.state
        let liveBatch = try ownerPipe.drain()
        let liveReadState = ownerPipe.readState
        fakeOwner.terminate()
        let exitDeadline = DispatchTime.now().uptimeNanoseconds + 2_000_000_000
        var closedBatch: [VideoControlEvent]?
        repeat {
            closedBatch = try ownerPipe.drain()
            if ownerPipe.readState == .ended { break }
            usleep(1000)
        } while DispatchTime.now().uptimeNanoseconds < exitDeadline
        let ownerStateBeforeWait = realOwner.state
        fakeOwner.waitUntilExit()
        assert(liveOwnerState == .alive && liveBatch?.isEmpty == true && liveReadState == .open)
        assert(closedBatch?.isEmpty == true && ownerPipe.readState == .ended)
        assert(ownerStateBeforeWait == .terminated, "EOF during parent exit must be recognized before explicit reaping")
        assert(realOwner.state == .terminated && videoControlEndFailure(parent: ownerStateBeforeWait) == nil)
        var freshness = VideoFrameFreshness()
        assert(freshness.accepts(sampleTime: time(0)))
        freshness.transition(paused: true, at: time(1))
        assert(!freshness.accepts(sampleTime: time(2)))
        freshness.transition(paused: false, at: time(3))
        assert(!freshness.accepts(sampleTime: time(2.9)), "queued paused sample must not leak after resume")
        assert(!freshness.accepts(sampleTime: CMTime(value: 30005, timescale: 10000)))
        assert(freshness.accepts(sampleTime: time(3.1)))
        var resumeRefresh = VideoResumeRefresh()
        resumeRefresh.transition(paused: true, sequence: 1)
        assert(resumeRefresh.takeRequest(available: true) == nil)
        resumeRefresh.transition(paused: false, sequence: 2)
        assert(resumeRefresh.takeRequest(available: false) == nil)
        assert(resumeRefresh.takeRequest(available: true) == 2, "static resume must request a fresh exact-window snapshot without a stream callback")
        assert(resumeRefresh.accepts(sequence: 2, currentSequence: 2, paused: false, requestedAt: time(3), cachedAt: nil))
        assert(!resumeRefresh.accepts(sequence: 2, currentSequence: 3, paused: true, requestedAt: time(3), cachedAt: nil))
        assert(!resumeRefresh.accepts(sequence: 2, currentSequence: 2, paused: false, requestedAt: time(3), cachedAt: time(4)), "late snapshot must not overwrite a newer stream frame")
        let timingCheckpoint = try VideoTimingCheckpoint(session: "session", partialPath: dir.appendingPathComponent("window.partial.mp4").path,
            nonce: "d0bee8db-9cf8-4fc3-94d9-e0c911e6db13", audioStart: 1000, first: 5000, pausedBeforeFirst: 1500,
            app: "Chrome", title: "Meet - checkpoint", width: 1920, height: 1080)
        assert(timingCheckpoint.offsetMs == 2500)
        let timingURL = dir.appendingPathComponent("timing.json")
        try timingCheckpoint.writeDurably(to: timingURL)
        let persisted = try JSONDecoder().decode(VideoTimingCheckpoint.self, from: Data(contentsOf: timingURL))
        assert(persisted.offsetMs == 2500 && persisted.captureNonce == timingCheckpoint.captureNonce)
        do { try timingCheckpoint.writeDurably(to: dir.appendingPathComponent("missing/timing.json")); assertionFailure("critical checkpoint failure must abort before appending video") } catch {}
        try Data("active".utf8).write(to: lease.url)
        let segmentSink = try VideoSegmentSink(lease: lease)
        let segmentedWriter = fragmentedVideoWriter(delegate: segmentSink)
        assert(segmentedWriter.outputFileTypeProfile == .mpeg4AppleHLS)
        assert(segmentedWriter.preferredOutputSegmentInterval.seconds == 10)
        assert(segmentedWriter.initialSegmentStartTime == .zero)
        segmentSink.assetWriter(segmentedWriter, didOutputSegmentData: Data("init".utf8), segmentType: .initialization, segmentReport: nil)
        let segmentBytes = try Data(contentsOf: lease.output)
        assert(segmentBytes == Data("init".utf8))
        assert(segmentSink.finish())
        try FileManager.default.removeItem(at: lease.url)
        lease.cleanupOutputs()
        segmentSink.assetWriter(segmentedWriter, didOutputSegmentData: Data("late".utf8), segmentType: .initialization, segmentReport: nil)
        assert(!FileManager.default.fileExists(atPath: lease.output.path))
        var delayedTimeline = VideoTimeline()
        var outputGate = VideoOutputGate()
        assert(outputGate.canAppend(delayedTimeline.outputTime(for: time(0))))
        outputGate.didAppend(time(0))
        assert(outputGate.canAppend(delayedTimeline.outputTime(for: time(2))))
        outputGate.didAppend(time(2))
        // These boundaries arrive only after the writer already committed output2s.
        delayedTimeline.pause(at: time(1)); delayedTimeline.resume(at: time(3))
        for host in [3.0, 4.0, 4.1] {
            assert(!outputGate.canAppend(delayedTimeline.outputTime(for: time(host))))
        }
        let caughtUp = delayedTimeline.outputTime(for: time(4.2))
        assert(outputGate.canAppend(caughtUp))
        outputGate.didAppend(caughtUp)
        assert(!outputGate.canAppend(time(2.3)))
        assert(delayedTimeline.pausedDuration.seconds == 2)
        var orderedTimeline = VideoTimeline()
        _ = orderedTimeline.outputTime(for: time(0))
        var controls = VideoControlState()
        let pause = VideoControlEvent(sequence: 1, paused: true, atHostMs: 10000)
        let resume = VideoControlEvent(sequence: 2, paused: false, atHostMs: 15000)
        // One coalesced notification delivers both transitions; reverse/stale notifications cannot re-pause.
        controls.reconcile([resume, pause], timeline: &orderedTimeline)
        controls.reconcile([pause], timeline: &orderedTimeline)
        controls.reconcile([pause, resume], timeline: &orderedTimeline)
        assert(!controls.paused && controls.appliedSequence == 2)
        assert(orderedTimeline.pausedDuration.seconds == 5)
        assert(orderedTimeline.outputTime(for: time(20)).seconds == 15)
        for mode in ["normal", "failure", "completed", "late_completed", "late_deadline"] {
            let status = dir.appendingPathComponent("shutdown-" + mode + ".json")
            let fragment = dir.appendingPathComponent("shutdown-" + mode + ".mp4")
            try Data("recording:none".utf8).write(to: status)
            let durableBytes = Data("already fsynced fragment model".utf8)
            try durableBytes.write(to: fragment)
            let file = try FileHandle(forWritingTo: fragment); try file.synchronize(); try file.close()
            let child = Process()
            child.executableURL = URL(fileURLWithPath: CommandLine.arguments[0])
            child.arguments = ["--hung-shutdown", status.path, mode]
            try child.run()
            let limit = DispatchTime.now().uptimeNanoseconds + 2_000_000_000
            while child.isRunning && DispatchTime.now().uptimeNanoseconds < limit { Thread.sleep(forTimeInterval: 0.01) }
            let exitedByDeadline = !child.isRunning
            if child.isRunning { child.terminate() }
            child.waitUntilExit()
            assert(exitedByDeadline && child.terminationStatus == 0, "independent deadline must terminate a hung shutdown")
            let retainedBytes = try Data(contentsOf: fragment)
            assert(retainedBytes == durableBytes, "forced exit must preserve already durable bytes")
            let expectedStatus: String
            switch mode {
            case "completed": expectedStatus = "stopped:none"
            case "late_completed": expectedStatus = "stopped:capture_failed"
            case "failure", "late_deadline": expectedStatus = "failed:capture_failed"
            default: expectedStatus = "recording:none"
            }
            let retainedStatus = try String(contentsOf: status, encoding: .utf8)
            assert(retainedStatus == expectedStatus, "normal crash remains recoverable; genuine failure stays failed")
        }
        var shutdownFailure = VideoShutdownFailure()
        assert(!shutdownFailure.record(nil))
        assert(shutdownFailure.resultReason(completed: true) == nil)
        assert(shutdownFailure.resultReason(completed: false) == "capture_failed")
        assert(shutdownFailure.record("permission_denied"))
        assert(!shutdownFailure.record("capture_failed"))
        assert(!shutdownFailure.record(nil))
        assert(shutdownFailure.resultReason(completed: true) == "permission_denied")
        let cancelledStatus = dir.appendingPathComponent("cancelled-shutdown")
        let cancelled = VideoShutdownDeadline(after: 0.05, reason: nil, publishFailure: { _ in assertionFailure() }, onTimeout: {
            try! Data("unexpected timeout".utf8).write(to: cancelledStatus)
        })
        cancelled.cancel()
        Thread.sleep(forTimeInterval: 0.1)
        assert(!FileManager.default.fileExists(atPath: cancelledStatus.path))
        print("Lease cleanup, normal keep, sequenced controls, sparse timing, cadence and bounded shutdown passed")
    }
}
