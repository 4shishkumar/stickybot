// C ABI over FluidAudio's Parakeet TDT v3 (CoreML; encoder on the Neural Engine by default).
// FluidAudio is async (actors); these entry points block the calling thread until done, which is
// what the Rust `Stt` trait wants (the app calls it from a dedicated worker thread).
import CoreML
import FluidAudio
import Foundation

private final class Engine {
    let asr: AsrManager
    init(asr: AsrManager) { self.asr = asr }
}

private final class Slot<T>: @unchecked Sendable {
    var value: T?
    var error: Error?
}

private func blocking<T>(_ body: @escaping @Sendable () async throws -> T) throws -> T {
    let sem = DispatchSemaphore(value: 0)
    let slot = Slot<T>()
    Task.detached {
        do { slot.value = try await body() } catch { slot.error = error }
        sem.signal()
    }
    sem.wait()
    if let e = slot.error { throw e }
    return slot.value!
}

private var lastError = ""

/// Last error message (valid until the next call). Never null.
@_cdecl("fluid_last_error")
public func fluid_last_error() -> UnsafePointer<CChar> {
    return (lastError as NSString).utf8String!
}

/// Downloads (first time) and loads Parakeet TDT v3 from `dir` (a folder FluidAudio fills).
/// `units`: 0 = FluidAudio default (CPU+ANE), 1 = CPU+GPU for the encoder.
/// Returns an opaque handle, or null on error (see fluid_last_error).
@_cdecl("fluid_load")
public func fluid_load(_ dir: UnsafePointer<CChar>, _ units: Int32) -> UnsafeMutableRawPointer? {
    let url = URL(fileURLWithPath: String(cString: dir)).appendingPathComponent("parakeet-tdt-0.6b-v3-coreml")
    do {
        let asr = try blocking { () async throws -> AsrManager in
            let models = try await AsrModels.downloadAndLoad(
                to: url, version: .v3, encoderComputeUnits: units == 1 ? .cpuAndGPU : nil)
            let asr = AsrManager(config: .default)
            try await asr.loadModels(models)
            return asr
        }
        return Unmanaged.passRetained(Engine(asr: asr)).toOpaque()
    } catch {
        lastError = "\(error)"
        return nil
    }
}

/// Transcribes `n` mono 16 kHz float samples. Returns a malloc'd UTF-8 string (free with
/// fluid_free_string), or null on error.
@_cdecl("fluid_transcribe")
public func fluid_transcribe(_ handle: UnsafeMutableRawPointer, _ samples: UnsafePointer<Float>, _ n: Int)
    -> UnsafeMutablePointer<CChar>?
{
    let engine = Unmanaged<Engine>.fromOpaque(handle).takeUnretainedValue()
    let audio = Array(UnsafeBufferPointer(start: samples, count: n))
    let asr = engine.asr
    do {
        let text = try blocking { () async throws -> String in
            var state = TdtDecoderState.make(decoderLayers: await asr.decoderLayerCount)
            return try await asr.transcribe(audio, decoderState: &state).text
        }
        return strdup(text)
    } catch {
        lastError = "\(error)"
        return nil
    }
}

@_cdecl("fluid_free_string")
public func fluid_free_string(_ s: UnsafeMutablePointer<CChar>?) {
    free(s)
}

@_cdecl("fluid_unload")
public func fluid_unload(_ handle: UnsafeMutableRawPointer) {
    Unmanaged<Engine>.fromOpaque(handle).release()
}
