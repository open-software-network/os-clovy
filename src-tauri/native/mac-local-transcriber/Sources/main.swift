import Foundation
import FluidAudio

// stdout belongs exclusively to the JSON-lines protocol, including library output.
let protocolOutput = FileHandle(fileDescriptor: dup(STDOUT_FILENO), closeOnDealloc: true)
dup2(STDERR_FILENO, STDOUT_FILENO)

struct Request: Decodable {
    let op: String
    let model_dir: String?
    let version: String?
    let audio_path: String?
    let language: String?
}

func respond(_ value: [String: Any]) {
    if var data = try? JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]) {
        data.append(10)
        protocolOutput.write(data)
    }
}

var manager: AsrManager?
var loadedDirectory: String?
while let line = readLine() {
    do {
        let request = try JSONDecoder().decode(Request.self, from: Data(line.utf8))
        switch request.op {
        case "load":
            guard request.version == "ultra", let directory = request.model_dir else {
                throw NSError(domain: "ClovyLocalTranscriber", code: 1, userInfo: [NSLocalizedDescriptionKey: "Expected an Ultra model directory"])
            }
            let start = ContinuousClock.now
            if manager == nil || loadedDirectory != directory {
                manager = nil
                loadedDirectory = nil
                // load(from:) may download missing components. loadLocal fails offline.
                let models = try AsrModels.loadLocal(from: URL(fileURLWithPath: directory), version: .ultra)
                let next = AsrManager()
                try await next.loadModels(models)
                manager = next
                loadedDirectory = directory
            }
            let elapsed = start.duration(to: .now).components
            respond(["ok": true, "load_ms": Double(elapsed.seconds) * 1000 + Double(elapsed.attoseconds) / 1e15])
        case "transcribe":
            guard let manager, let path = request.audio_path else {
                throw NSError(domain: "ClovyLocalTranscriber", code: 2, userInfo: [NSLocalizedDescriptionKey: "Load the model before transcribing"])
            }
            var state = try TdtDecoderState()
            let language = request.language.flatMap(Language.init(rawValue:))
            let result = try await manager.transcribe(URL(fileURLWithPath: path), decoderState: &state, language: language)
            let durationMs = Int((result.duration * 1000).rounded())
            // TDT timings are encoder-frame aligned and may extend into final padding.
            let tokens: [[String: Any]] = (result.tokenTimings ?? []).map {
                let start = min(durationMs, max(0, Int(($0.startTime * 1000).rounded())))
                let end = min(durationMs, max(start, Int(($0.endTime * 1000).rounded())))
                return ["t": $0.token, "start_ms": start, "end_ms": end]
            }
            respond(["ok": true, "text": result.text, "tokens": tokens, "duration_ms": durationMs, "language": language?.rawValue as Any? ?? NSNull()])
        case "unload":
            manager = nil
            loadedDirectory = nil
            respond(["ok": true])
        default:
            respond(["ok": false, "error": "Unknown operation"])
        }
    } catch {
        respond(["ok": false, "error": error.localizedDescription])
    }
}
