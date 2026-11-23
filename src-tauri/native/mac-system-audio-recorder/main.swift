import AVFoundation
import AppKit
import AudioToolbox
import CoreAudio
import Darwin
import Foundation
import IOKit.pwr_mgt

private let zeroSignalRMSFloor: Float = 1.0e-7
private let startupZeroSignalRecoverySeconds: TimeInterval = 10
private let activeZeroSignalRecoverySeconds: TimeInterval = 30

extension String: @retroactive Error {}

extension String: @retroactive LocalizedError {
    public var errorDescription: String? { self }
}

extension AudioObjectID {
    static let system = AudioObjectID(kAudioObjectSystemObject)
    static let unknown = kAudioObjectUnknown
    var isValid: Bool { self != .unknown }

    static func readDefaultSystemOutputDevice() throws -> AudioDeviceID {
        try AudioObjectID.system.read(
            kAudioHardwarePropertyDefaultSystemOutputDevice,
            defaultValue: AudioDeviceID.unknown
        )
    }

    static func readAudioDevices() throws -> [AudioDeviceID] {
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioHardwarePropertyDevices,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        var dataSize: UInt32 = 0
        var err = AudioObjectGetPropertyDataSize(AudioObjectID.system, &address, 0, nil, &dataSize)
        guard err == noErr else { throw "Error reading audio device list size: \(err)" }
        let count = Int(dataSize) / MemoryLayout<AudioDeviceID>.size
        guard count > 0 else { return [] }
        var devices = Array(repeating: AudioDeviceID.unknown, count: count)
        err = devices.withUnsafeMutableBufferPointer { pointer in
            AudioObjectGetPropertyData(AudioObjectID.system, &address, 0, nil, &dataSize, pointer.baseAddress!)
        }
        guard err == noErr else { throw "Error reading audio device list: \(err)" }
        return devices.filter { $0 != AudioDeviceID.unknown }
    }

    func readName() throws -> String {
        try readString(kAudioObjectPropertyName)
    }

    func readDeviceUID() throws -> String {
        try readString(kAudioDevicePropertyDeviceUID)
    }

    func readAudioTapStreamBasicDescription() throws -> AudioStreamBasicDescription {
        try read(kAudioTapPropertyFormat, defaultValue: AudioStreamBasicDescription())
    }

    func readString(_ selector: AudioObjectPropertySelector) throws -> String {
        try read(AudioObjectPropertyAddress(mSelector: selector, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain), defaultValue: "" as CFString) as String
    }

    func read<T>(_ selector: AudioObjectPropertySelector, defaultValue: T) throws -> T {
        try read(AudioObjectPropertyAddress(mSelector: selector, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain), defaultValue: defaultValue)
    }

    func read<T>(_ address: AudioObjectPropertyAddress, defaultValue: T) throws -> T {
        var address = address
        var dataSize: UInt32 = 0
        var err = AudioObjectGetPropertyDataSize(self, &address, 0, nil, &dataSize)
        guard err == noErr else { throw "Error reading data size for audio property: \(err)" }

        var value = defaultValue
        err = withUnsafeMutablePointer(to: &value) { pointer in
            AudioObjectGetPropertyData(self, &address, 0, nil, &dataSize, pointer)
        }
        guard err == noErr else { throw "Error reading audio property: \(err)" }
        return value
    }
}

final class SystemAudioRecorder {
    private let outputURL: URL?
    private let statusURL: URL?
    private let admissionURL: URL?
    private var admissionConsumed = false
    private let pidURL: URL?
    private let logURL: URL?
    private let timelineOffset: TimeInterval
    private let pauseLock = NSLock()
    private let healthLock = NSLock()
    private let rebuildLock = NSLock()

    private var processTapID = AudioObjectID.unknown
    private var aggregateDeviceID = AudioObjectID.unknown
    private var deviceProcID: AudioDeviceIOProcID?
    private var audioFile: AVAudioFile?
    private var audioConverter: AVAudioConverter?
    private var inputFormat: AVAudioFormat?
    private var outputFormat: AVAudioFormat?
    private var fileFormat: AVAudioFormat?
    private var didStop = false
    private var isPaused = false
    private var activeStartedAt: Date?
    private var pausedAt: Date?
    private var accumulatedPausedDuration: TimeInterval = 0
    private var outputFramesWritten: AVAudioFramePosition = 0
    private var lastLevelEmit = Date.distantPast
    private var lastInputAt = Date.distantPast
    private var graphStartedAt = Date.distantPast
    private var lastSignalAt = Date.distantPast
    private var observedSignalSinceGraphStart = false
    private var attemptedGraphRecovery = false
    private var reportedTerminalInputStall = false
    private var maxLevel: Double = 0
    private var stallTimer: DispatchSourceTimer?
    private var powerAssertionID: IOPMAssertionID = 0
    private var rebuildScheduled = false

    init(outputURL: URL?, statusURL: URL?, pidURL: URL?, logURL: URL?, timelineOffset: TimeInterval, admissionURL: URL? = nil) {
        self.outputURL = outputURL
        self.statusURL = statusURL
        self.admissionURL = admissionURL
        self.pidURL = pidURL
        self.logURL = logURL
        self.timelineOffset = timelineOffset
    }

    func writePid() {
        guard let pidURL else { return }
        try? "\(getpid())".write(to: pidURL, atomically: true, encoding: .utf8)
        log("wrote pid \(getpid()) to \(pidURL.path)")
    }

    func pause() {
        pauseLock.lock()
        if !isPaused {
            pausedAt = Date()
        }
        isPaused = true
        pauseLock.unlock()
        emit(["event": "paused"])
    }

    func resume() {
        pauseLock.lock()
        if let pausedAt {
            accumulatedPausedDuration += Date().timeIntervalSince(pausedAt)
        }
        pausedAt = nil
        isPaused = false
        pauseLock.unlock()
        emit(["event": "resumed"])
    }

    func start(checkOnly: Bool = false) throws {
        log("starting; output=\(outputURL?.path ?? "check") status=\(statusURL?.path ?? "none")")
        try ensureSystemAudioPermission(logURL: logURL)
        if let outputURL {
            try? FileManager.default.removeItem(at: outputURL)
            try FileManager.default.createDirectory(at: outputURL.deletingLastPathComponent(), withIntermediateDirectories: true)
        }
        cleanupStaleAggregateDevices(named: "Clovy System Audio")
        if !checkOnly {
            acquirePowerAssertion()
        }
        activeStartedAt = Date().addingTimeInterval(-timelineOffset)
        accumulatedPausedDuration = 0
        pausedAt = nil
        outputFramesWritten = 0
        resetSignalHealthForRecording()
        markInputReceived()

        try startAudioGraph(checkOnly: checkOnly)
        if checkOnly {
            emit(["event": "authorized", "message": "System audio capture is authorized."])
            return
        }
        startStallWatchdog()
        emit(["event": "ready", "output": outputURL?.path ?? "check"])
    }

    private func startAudioGraph(checkOnly: Bool = false) throws {
        teardownAudioGraph()

        // Bind the tap to the output stream the user is actually hearing. This
        // removes global-tap ambiguity when apps or output routes select a
        // concrete device stream. The tap remains the aggregate's only input
        // source; the physical output device must not be added as a sub-device.
        let systemOutputID = try AudioObjectID.readDefaultSystemOutputDevice()
        guard systemOutputID.isValid else {
            throw "The default system output device is unavailable."
        }
        let outputUID = try systemOutputID.readDeviceUID()
        log("default output device id=\(systemOutputID) uid=\(outputUID)")

        let tapDescription = CATapDescription(
            excludingProcesses: [],
            deviceUID: outputUID,
            stream: 0
        )
        tapDescription.uuid = UUID()
        tapDescription.isMixdown = true
        tapDescription.isMono = false
        tapDescription.isExclusive = true
        tapDescription.isPrivate = true
        tapDescription.muteBehavior = .unmuted
        tapDescription.name = "Clovy System Audio"

        var tapID = AudioObjectID.unknown
        var err = AudioHardwareCreateProcessTap(tapDescription, &tapID)
        guard err == noErr else {
            log("AudioHardwareCreateProcessTap failed err=\(err)")
            throw "System audio permission or tap creation failed with error \(err)"
        }
        log("created process tap id=\(tapID)")
        processTapID = tapID

        var streamDescription = try tapID.readAudioTapStreamBasicDescription()
        log("tap format \(describeAudioFormat(streamDescription))")
        guard let inputFormat = AVAudioFormat(streamDescription: &streamDescription) else {
            throw "Failed to create audio format for system tap: \(describeAudioFormat(streamDescription))."
        }
        let targetFormat: AVAudioFormat
        if let fileFormat {
            targetFormat = fileFormat
        } else if let initialFormat = AVAudioFormat(commonFormat: .pcmFormatInt16, sampleRate: inputFormat.sampleRate, channels: inputFormat.channelCount, interleaved: true) {
            targetFormat = initialFormat
            fileFormat = initialFormat
        } else {
            throw "Failed to create output audio format."
        }
        guard let converter = AVAudioConverter(from: inputFormat, to: targetFormat) else {
            throw "Failed to create audio converter."
        }

        let aggregateUID = UUID().uuidString
        let description: [String: Any] = [
            kAudioAggregateDeviceNameKey: "Clovy System Audio",
            kAudioAggregateDeviceUIDKey: aggregateUID,
            kAudioAggregateDeviceIsPrivateKey: true,
            kAudioAggregateDeviceIsStackedKey: false,
            kAudioAggregateDeviceTapAutoStartKey: true,
            kAudioAggregateDeviceTapListKey: [
                [
                    kAudioSubTapDriftCompensationKey: true,
                    kAudioSubTapUIDKey: tapDescription.uuid.uuidString
                ]
            ]
        ]

        err = AudioHardwareCreateAggregateDevice(description as CFDictionary, &aggregateDeviceID)
        guard err == noErr else {
            log("AudioHardwareCreateAggregateDevice failed err=\(err)")
            throw "Failed to create aggregate audio device: \(err)"
        }
        log("created aggregate device id=\(aggregateDeviceID)")
        try waitForAggregateDeviceReady(aggregateDeviceID)

        if checkOnly {
            return
        }

        self.inputFormat = inputFormat
        self.outputFormat = targetFormat
        audioConverter = converter
        if audioFile == nil, let outputURL {
            audioFile = try AVAudioFile(forWriting: outputURL, settings: targetFormat.settings, commonFormat: .pcmFormatInt16, interleaved: true)
        }

        log("creating IO callback")
        err = AudioDeviceCreateIOProcID(
            aggregateDeviceID,
            systemAudioIOProc,
            Unmanaged.passUnretained(self).toOpaque(),
            &deviceProcID
        )
        guard err == noErr else {
            log("AudioDeviceCreateIOProcID failed err=\(err)")
            throw "Failed to create audio IO callback: \(err)"
        }
        log("created IO callback")

        // Agent starts prepare all CoreAudio resources first. Only the host
        // can release this one-shot gate after revalidating its live turn.
        if let admissionURL, !admissionConsumed {
            emit(["event": "prepared", "output": outputURL?.path ?? "check"])
            let deadline = Date().addingTimeInterval(30)
            while !FileManager.default.fileExists(atPath: admissionURL.path) {
                guard Date() < deadline else { throw "System audio host admission expired." }
                // Do not re-enter graph rebuild callbacks before admission.
                Thread.sleep(forTimeInterval: 0.01)
            }
            try FileManager.default.removeItem(at: admissionURL)
            admissionConsumed = true
        }

        err = AudioDeviceStart(aggregateDeviceID, deviceProcID)
        guard err == noErr else {
            log("AudioDeviceStart failed err=\(err)")
            throw "Failed to start system audio capture: \(err)"
        }
        log("audio device started")
        markGraphStarted()
        markInputReceived()
    }

    func stop(emitStopped: Bool = true) {
        guard !didStop else { return }
        didStop = true
        stopStallWatchdog()
        flushTimelineSilenceToNow()
        teardownAudioGraph()
        audioFile = nil
        audioConverter = nil
        inputFormat = nil
        outputFormat = nil
        fileFormat = nil
        releasePowerAssertion()
        log("stopped maxLevel=\(maxLevel)")
        if emitStopped {
            emit(["event": "stopped", "output": outputURL?.path ?? "check", "maxLevel": String(maxLevel)])
        }
    }

    private func teardownAudioGraph() {
        if aggregateDeviceID.isValid {
            AudioDeviceStop(aggregateDeviceID, deviceProcID)
            if let deviceProcID {
                AudioDeviceDestroyIOProcID(aggregateDeviceID, deviceProcID)
            }
            AudioHardwareDestroyAggregateDevice(aggregateDeviceID)
        }
        if processTapID.isValid {
            AudioHardwareDestroyProcessTap(processTapID)
        }
        aggregateDeviceID = .unknown
        processTapID = .unknown
        deviceProcID = nil
        audioConverter = nil
        inputFormat = nil
        outputFormat = nil
    }

    fileprivate func rebuildAudioGraph(reason: String) {
        rebuildLock.lock()
        guard !didStop, !rebuildScheduled else {
            rebuildLock.unlock()
            return
        }
        rebuildScheduled = true
        rebuildLock.unlock()
        markInputReceived()
        DispatchQueue.main.async {
            defer { self.clearRebuildScheduled() }
            guard !self.didStop else { return }
            self.log("rebuilding audio graph: \(reason)")
            self.emit(["event": "restarting", "message": reason])
            do {
                try self.startAudioGraph()
                self.emit(["event": "ready", "output": self.outputURL?.path ?? "check"])
            } catch {
                let message = "System audio capture stopped: \(describeError(error))"
                self.log(message)
                self.emit(["event": "error", "message": message])
            }
        }
    }

    private func clearRebuildScheduled() {
        rebuildLock.lock()
        rebuildScheduled = false
        rebuildLock.unlock()
    }

    private func emitLevel(from buffer: AVAudioPCMBuffer) {
        let now = Date()
        guard now.timeIntervalSince(lastLevelEmit) >= 0.08 else { return }
        lastLevelEmit = now
        let frameLength = Int(buffer.frameLength)
        let channelCount = Int(buffer.format.channelCount)
        guard frameLength > 0, channelCount > 0, let channels = buffer.floatChannelData else {
            emit(["event": "level", "level": "0"])
            return
        }
        var sum: Float = 0
        var count = 0
        for channelIndex in 0..<channelCount {
            let channel = channels[channelIndex]
            for frameIndex in 0..<frameLength {
                let sample = channel[frameIndex]
                sum += sample * sample
                count += 1
            }
        }
        let rms = count > 0 ? sqrt(sum / Float(count)) : 0
        if rms > zeroSignalRMSFloor {
            markSignalReceived()
        }
        let level = min(1, Double(rms) * 4)
        maxLevel = max(maxLevel, level)
        emit(["event": "level", "level": String(level), "maxLevel": String(maxLevel)])
    }

    private func markInputReceived() {
        healthLock.lock()
        lastInputAt = Date()
        healthLock.unlock()
    }

    private func resetSignalHealthForRecording() {
        healthLock.lock()
        graphStartedAt = Date.distantPast
        lastSignalAt = Date.distantPast
        observedSignalSinceGraphStart = false
        attemptedGraphRecovery = false
        reportedTerminalInputStall = false
        healthLock.unlock()
    }

    private func markGraphStarted() {
        healthLock.lock()
        graphStartedAt = Date()
        lastSignalAt = Date.distantPast
        observedSignalSinceGraphStart = false
        healthLock.unlock()
    }

    private func markSignalReceived() {
        healthLock.lock()
        lastSignalAt = Date()
        observedSignalSinceGraphStart = true
        healthLock.unlock()
    }

    /// CoreAudio process taps can keep invoking the IO callback while returning
    /// only zero-filled buffers. One full graph rebuild is safe during real
    /// silence and recovers that state without sacrificing the saved timeline.
    private func zeroSignalRecoveryReason() -> String? {
        healthLock.lock()
        defer { healthLock.unlock() }
        guard !attemptedGraphRecovery else { return nil }
        let now = Date()
        if !observedSignalSinceGraphStart,
           now.timeIntervalSince(graphStartedAt) >= startupZeroSignalRecoverySeconds {
            attemptedGraphRecovery = true
            return "System audio capture returned only zero-filled buffers after startup; restarting capture."
        }
        if observedSignalSinceGraphStart,
           now.timeIntervalSince(lastSignalAt) >= activeZeroSignalRecoverySeconds {
            attemptedGraphRecovery = true
            return "System audio capture stopped returning signal; restarting capture."
        }
        return nil
    }

    /// After the one permitted graph rebuild, continued zero-filled buffers
    /// are terminal for the system source even though CoreAudio is still
    /// invoking the callback. Report that state instead of silently padding a
    /// meeting-length WAV with zeroes.
    private func terminalZeroSignalMessage() -> String? {
        healthLock.lock()
        defer { healthLock.unlock() }
        guard attemptedGraphRecovery, !reportedTerminalInputStall else { return nil }
        let now = Date()
        let terminal = if !observedSignalSinceGraphStart {
            now.timeIntervalSince(graphStartedAt) >= startupZeroSignalRecoverySeconds
        } else {
            now.timeIntervalSince(lastSignalAt) >= activeZeroSignalRecoverySeconds
        }
        guard terminal else { return nil }
        reportedTerminalInputStall = true
        return "System audio capture is still returning silence after one restart. Microphone recording can continue."
    }

    private func secondsSinceLastInput() -> TimeInterval {
        healthLock.lock()
        let elapsed = Date().timeIntervalSince(lastInputAt)
        healthLock.unlock()
        return elapsed
    }

    private func claimInputStallRecovery() -> Bool {
        healthLock.lock()
        defer { healthLock.unlock() }
        guard !attemptedGraphRecovery else { return false }
        attemptedGraphRecovery = true
        return true
    }

    private func claimTerminalInputStallReport() -> Bool {
        healthLock.lock()
        defer { healthLock.unlock() }
        guard !reportedTerminalInputStall else { return false }
        reportedTerminalInputStall = true
        return true
    }

    private func startStallWatchdog() {
        stopStallWatchdog()
        let timer = DispatchSource.makeTimerSource(queue: .main)
        timer.schedule(deadline: .now() + 3, repeating: 1)
        timer.setEventHandler { [weak self] in
            guard let self, !self.didStop else { return }
            self.pauseLock.lock()
            let paused = self.isPaused
            self.pauseLock.unlock()
            guard !paused else { return }
            let stalledFor = self.secondsSinceLastInput()
            if stalledFor >= 3 {
                if self.claimInputStallRecovery() {
                    let message = "System audio capture stalled for \(String(format: "%.1f", stalledFor))s; restarting capture once."
                    self.log(message)
                    self.emit(["event": "stalled", "message": message])
                    self.rebuildAudioGraph(reason: message)
                } else if self.claimTerminalInputStallReport() {
                    let message = "System audio capture did not resume after one restart. Microphone recording can continue."
                    self.log(message)
                    self.emit(["event": "error", "message": message])
                }
                return
            }
            if let message = self.zeroSignalRecoveryReason() {
                self.log(message)
                self.emit(["event": "stalled", "message": message])
                self.rebuildAudioGraph(reason: message)
            } else if let message = self.terminalZeroSignalMessage() {
                self.log(message)
                self.emit(["event": "error", "message": message])
            }
        }
        stallTimer = timer
        timer.resume()
    }

    private func stopStallWatchdog() {
        stallTimer?.cancel()
        stallTimer = nil
    }

    private func acquirePowerAssertion() {
        guard powerAssertionID == 0 else { return }
        var assertionID = IOPMAssertionID(0)
        let reason = "Clovy recording in progress" as CFString
        let result = IOPMAssertionCreateWithName(
            kIOPMAssertionTypeNoDisplaySleep as CFString,
            IOPMAssertionLevel(kIOPMAssertionLevelOn),
            reason,
            &assertionID
        )
        if result == kIOReturnSuccess {
            powerAssertionID = assertionID
            log("created power assertion id=\(assertionID)")
        } else {
            log("power assertion failed result=\(result)")
        }
    }

    private func releasePowerAssertion() {
        guard powerAssertionID != 0 else { return }
        let result = IOPMAssertionRelease(powerAssertionID)
        log("released power assertion id=\(powerAssertionID) result=\(result)")
        powerAssertionID = 0
    }

    private func flushTimelineSilenceToNow() {
        guard let audioFile, let outputFormat else { return }
        do {
            try writeTimelineSilenceIfNeeded(beforeWriting: 0, to: audioFile, format: outputFormat)
        } catch {
            let message = "Failed to write trailing system-audio silence: \(describeError(error))"
            log(message)
            emit(["event": "error", "message": message])
        }
    }

    private func emit(_ object: [String: String]) {
        let data = try! JSONSerialization.data(withJSONObject: object)
        print(String(data: data, encoding: .utf8)!)
        fflush(stdout)
        guard let statusURL else { return }
        try? data.write(to: statusURL)
    }

    private func log(_ message: String) {
        writeLog(message, logURL: logURL)
    }

    fileprivate func handleInputData(_ inputData: UnsafePointer<AudioBufferList>) {
        pauseLock.lock()
        let paused = isPaused
        pauseLock.unlock()
        guard !paused else { return }
        markInputReceived()
        guard let inputFormat, let outputFormat, let converter = audioConverter else { return }
        guard let buffer = AVAudioPCMBuffer(pcmFormat: inputFormat, bufferListNoCopy: inputData, deallocator: nil) else { return }
        do {
            emitLevel(from: buffer)
            let frameCapacity = max(1, AVAudioFrameCount(Double(buffer.frameLength) * outputFormat.sampleRate / inputFormat.sampleRate))
            guard let convertedBuffer = AVAudioPCMBuffer(pcmFormat: outputFormat, frameCapacity: frameCapacity) else { return }
            var didProvideInput = false
            var conversionError: NSError?
            let status = converter.convert(to: convertedBuffer, error: &conversionError) { _, inputStatus in
                if didProvideInput {
                    inputStatus.pointee = .noDataNow
                    return nil
                }
                didProvideInput = true
                inputStatus.pointee = .haveData
                return buffer
            }
            if let conversionError { throw conversionError }
            if status == .haveData || status == .inputRanDry, convertedBuffer.frameLength > 0, let audioFile {
                try writeTimelineSilenceIfNeeded(beforeWriting: convertedBuffer.frameLength, to: audioFile, format: outputFormat)
                try audioFile.write(from: convertedBuffer)
                outputFramesWritten += AVAudioFramePosition(convertedBuffer.frameLength)
            }
        } catch {
            emit(["event": "error", "message": describeError(error)])
        }
    }

    private func writeTimelineSilenceIfNeeded(beforeWriting incomingFrames: AVAudioFrameCount, to audioFile: AVAudioFile, format: AVAudioFormat) throws {
        guard let activeStartedAt else { return }
        pauseLock.lock()
        let activePauseDuration = isPaused ? pausedAt.map { Date().timeIntervalSince($0) } ?? 0 : 0
        let pausedOffset = accumulatedPausedDuration + activePauseDuration
        pauseLock.unlock()
        let activeElapsed = max(0, Date().timeIntervalSince(activeStartedAt) - pausedOffset)
        let expectedFrames = AVAudioFramePosition(activeElapsed * format.sampleRate)
        var missingFrames = expectedFrames - outputFramesWritten - AVAudioFramePosition(incomingFrames)
        let toleranceFrames = AVAudioFramePosition(format.sampleRate * 0.08)
        guard missingFrames > toleranceFrames else { return }
        while missingFrames > 0 {
            let chunkFrames = AVAudioFrameCount(min(missingFrames, AVAudioFramePosition(format.sampleRate / 2)))
            guard let silence = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: chunkFrames) else { return }
            silence.frameLength = chunkFrames
            zeroAudioBuffer(silence)
            try audioFile.write(from: silence)
            outputFramesWritten += AVAudioFramePosition(chunkFrames)
            missingFrames -= AVAudioFramePosition(chunkFrames)
        }
    }

    private func zeroAudioBuffer(_ buffer: AVAudioPCMBuffer) {
        let audioBuffers = UnsafeMutableAudioBufferListPointer(buffer.mutableAudioBufferList)
        for audioBuffer in audioBuffers {
            guard let data = audioBuffer.mData else { continue }
            memset(data, 0, Int(audioBuffer.mDataByteSize))
        }
    }

    private func cleanupStaleAggregateDevices(named targetName: String) {
        do {
            for device in try AudioObjectID.readAudioDevices() {
                guard (try? AudioObjectID(device).readName()) == targetName else { continue }
                let err = AudioHardwareDestroyAggregateDevice(device)
                log("destroy stale aggregate device id=\(device) err=\(err)")
            }
        } catch {
            log("stale aggregate cleanup failed: \(describeError(error))")
        }
    }

    private func waitForAggregateDeviceReady(_ deviceID: AudioObjectID) throws {
        let deadline = Date().addingTimeInterval(3)
        var lastError: Error?
        while Date() < deadline {
            do {
                _ = try deviceID.readName()
                log("aggregate device is readable")
                return
            } catch {
                lastError = error
                Thread.sleep(forTimeInterval: 0.1)
            }
        }
        throw "Aggregate audio device was not readable after creation: \(lastError.map(describeError) ?? "unknown error")"
    }
}

private let systemAudioIOProc: AudioDeviceIOProc = { _, _, inputData, _, _, _, clientData in
    guard let clientData else { return noErr }
    let recorder = Unmanaged<SystemAudioRecorder>.fromOpaque(clientData).takeUnretainedValue()
    recorder.handleInputData(inputData)
    return noErr
}

private enum SystemAudioPermissionStatus {
    case authorized
    case denied
    case unknown
}

private typealias TCCPreflightFunction = @convention(c) (CFString, CFDictionary?) -> Int32
private typealias TCCRequestFunction = @convention(c) (CFString, CFDictionary?, @escaping (Bool) -> Void) -> Void

private func ensureSystemAudioPermission(logURL: URL?) throws {
    guard let preflight = loadTCCFunction("TCCAccessPreflight", as: TCCPreflightFunction.self, logURL: logURL) else {
        writeLog("TCC preflight SPI is unavailable; continuing with CoreAudio permission behavior", logURL: logURL)
        return
    }

    let status = systemAudioPermissionStatus(preflight("kTCCServiceAudioCapture" as CFString, nil))
    writeLog("system audio permission preflight status=\(status)", logURL: logURL)
    switch status {
    case .authorized:
        return
    case .denied:
        throw "System Audio Recording permission is denied. Enable Clovy in System Settings > Privacy & Security > Screen & System Audio Recording."
    case .unknown:
        break
    }

    guard let request = loadTCCFunction("TCCAccessRequest", as: TCCRequestFunction.self, logURL: logURL) else {
        throw "System Audio Recording permission has not been granted, and macOS did not expose a permission request API."
    }

    writeLog("requesting system audio permission", logURL: logURL)
    NSApplication.shared.setActivationPolicy(.accessory)
    NSApplication.shared.finishLaunching()
    NSApplication.shared.activate(ignoringOtherApps: true)
    var granted = false
    var completed = false
    request("kTCCServiceAudioCapture" as CFString, nil) { allowed in
        granted = allowed
        completed = true
    }

    let deadline = Date().addingTimeInterval(60)
    while !completed && Date() < deadline {
        RunLoop.current.run(mode: .default, before: Date().addingTimeInterval(0.1))
    }

    if !completed {
        throw "Timed out waiting for System Audio Recording permission. Check the macOS permission prompt and try again."
    }
    writeLog("system audio permission request granted=\(granted)", logURL: logURL)
    guard granted else {
        throw "System Audio Recording permission was not granted. Enable Clovy in System Settings > Privacy & Security > Screen & System Audio Recording."
    }
}

private func systemAudioPermissionStatus(_ rawStatus: Int32) -> SystemAudioPermissionStatus {
    if rawStatus == 0 { return .authorized }
    if rawStatus == 1 { return .denied }
    return .unknown
}

private func describeError(_ error: Error) -> String {
    if let message = error as? String {
        return message
    }
    return error.localizedDescription
}

private func describeAudioFormat(_ description: AudioStreamBasicDescription) -> String {
    [
        "sampleRate=\(description.mSampleRate)",
        "formatID=\(description.mFormatID)",
        "flags=\(description.mFormatFlags)",
        "bytesPerPacket=\(description.mBytesPerPacket)",
        "framesPerPacket=\(description.mFramesPerPacket)",
        "bytesPerFrame=\(description.mBytesPerFrame)",
        "channels=\(description.mChannelsPerFrame)",
        "bits=\(description.mBitsPerChannel)",
    ].joined(separator: " ")
}

private func loadTCCFunction<T>(_ name: String, as type: T.Type, logURL: URL?) -> T? {
    let tccPath = "/System/Library/PrivateFrameworks/TCC.framework/Versions/A/TCC"
    guard let handle = dlopen(tccPath, RTLD_NOW) else {
        writeLog("dlopen TCC failed for \(name)", logURL: logURL)
        return nil
    }
    guard let symbol = dlsym(handle, name) else {
        writeLog("dlsym TCC failed for \(name)", logURL: logURL)
        return nil
    }
    return unsafeBitCast(symbol, to: type)
}

func argumentValue(_ name: String, from arguments: [String]) -> String? {
    guard let index = arguments.firstIndex(of: name), arguments.indices.contains(index + 1) else {
        return nil
    }
    return arguments[index + 1]
}

func emitProcessStatus(_ object: [String: String], statusPath: String?) {
    let data = try! JSONSerialization.data(withJSONObject: object)
    print(String(data: data, encoding: .utf8)!)
    fflush(stdout)
    guard let statusPath else { return }
    try? data.write(to: URL(fileURLWithPath: statusPath))
}

// This probe exits before permission checks, graph setup or capture. Older
// helpers reject it without --output, allowing the host to fail closed.
if CommandLine.arguments.contains("--admission-protocol") {
    print("1")
    exit(0)
}

let statusPath = argumentValue("--status", from: CommandLine.arguments)
let logPath = argumentValue("--log", from: CommandLine.arguments)

func writeLog(_ message: String, logURL: URL?) {
    guard let logURL else { return }
    let line = "\(Date()) pid=\(getpid()) \(message)\n"
    if FileManager.default.fileExists(atPath: logURL.path), let handle = try? FileHandle(forWritingTo: logURL) {
        defer { try? handle.close() }
        _ = try? handle.seekToEnd()
        try? handle.write(contentsOf: Data(line.utf8))
    } else {
        try? line.write(to: logURL, atomically: true, encoding: .utf8)
    }
}

writeLog("launched args=\(CommandLine.arguments.joined(separator: " "))", logURL: logPath.map { URL(fileURLWithPath: $0) })

guard #available(macOS 14.2, *) else {
    writeLog("unsupported macOS version", logURL: logPath.map { URL(fileURLWithPath: $0) })
    emitProcessStatus(["event": "error", "message": "System audio recording requires macOS 14.2 or later."], statusPath: statusPath)
    exit(2)
}

let checkOnly = CommandLine.arguments.contains("--check")
let outputPath = argumentValue("--output", from: CommandLine.arguments)
let pidPath = argumentValue("--pid", from: CommandLine.arguments)
let timelineOffsetMs = argumentValue("--timeline-offset-ms", from: CommandLine.arguments).flatMap(Double.init) ?? 0
if !checkOnly && outputPath == nil {
    writeLog("missing output argument", logURL: logPath.map { URL(fileURLWithPath: $0) })
    emitProcessStatus(["event": "error", "message": "Usage: clovy-system-audio-recorder --output /path/to/recording.wav"], statusPath: statusPath)
    exit(2)
}

let helperLogURL = logPath.map { URL(fileURLWithPath: $0) }
let recorder = SystemAudioRecorder(
    outputURL: outputPath.map { URL(fileURLWithPath: $0) },
    statusURL: statusPath.map { URL(fileURLWithPath: $0) },
    pidURL: pidPath.map { URL(fileURLWithPath: $0) },
    logURL: helperLogURL,
    timelineOffset: max(0, timelineOffsetMs) / 1000,
    admissionURL: argumentValue("--admission", from: CommandLine.arguments).map { URL(fileURLWithPath: $0) }
)
recorder.writePid()

let terminateSource = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
let interruptSource = DispatchSource.makeSignalSource(signal: SIGINT, queue: .main)
let pauseSource = DispatchSource.makeSignalSource(signal: SIGUSR1, queue: .main)
let resumeSource = DispatchSource.makeSignalSource(signal: SIGUSR2, queue: .main)
signal(SIGTERM, SIG_IGN)
signal(SIGINT, SIG_IGN)
signal(SIGUSR1, SIG_IGN)
signal(SIGUSR2, SIG_IGN)

terminateSource.setEventHandler {
    recorder.stop()
    exit(0)
}
interruptSource.setEventHandler {
    recorder.stop()
    exit(0)
}
pauseSource.setEventHandler {
    recorder.pause()
}
resumeSource.setEventHandler {
    recorder.resume()
}
terminateSource.resume()
interruptSource.resume()
pauseSource.resume()
resumeSource.resume()

do {
    try recorder.start(checkOnly: checkOnly)
    if checkOnly {
        recorder.stop(emitStopped: false)
        exit(0)
    }
} catch {
    let message = describeError(error)
    writeLog("start failed: \(message)", logURL: helperLogURL)
    emitProcessStatus(["event": "error", "message": message], statusPath: statusPath)
    exit(1)
}

dispatchMain()
