import AVFoundation
import Foundation
import RomlensKit
import Synchronization

/// Stereo 16-bit samples from one producer to one consumer, without locks:
/// the producer renders on a background queue, the audio thread reads.
/// Nothing on the audio thread allocates, locks or calls into Rust
/// (docs/23, A12).
///
/// Only the consumer moves `read`. A clear is a request: the main actor
/// bumps `requested`, and the producer tags what it writes with the
/// request its player was started under, marking where that player's
/// samples begin. From the request the consumer plays silence until the
/// mark, then skips to it, so what an old player rendered is never heard.
final class SampleRing: @unchecked Sendable {
    /// Frames it holds: 0.25 s at 32 kHz.
    let capacity: Int
    private let buffer: UnsafeMutablePointer<Int16>
    private let written = Atomic<Int>(0)
    private let read = Atomic<Int>(0)
    /// The latest clear asked for (main actor).
    private let requested = Atomic<Int>(0)
    /// Where the samples of generation `startGeneration` begin (producer).
    private let startAt = Atomic<Int>(0)
    private let startGeneration = Atomic<Int>(0)
    /// The generation the producer last wrote (producer only).
    private let writing = Atomic<Int>(0)
    /// The generation the consumer plays (consumer only).
    private let serving = Atomic<Int>(0)

    init(capacity: Int = 8192) {
        self.capacity = capacity
        buffer = .allocate(capacity: capacity * 2)
        buffer.initialize(repeating: 0, count: capacity * 2)
    }

    deinit { buffer.deallocate() }

    /// Frames waiting to be read.
    var available: Int { written.load(ordering: .acquiring) - read.load(ordering: .acquiring) }

    var space: Int { capacity - available }

    /// Frames of `generation` waiting to be read, as the producer sees
    /// them: an older player's left in the ring do not count.
    func pending(generation: Int) -> Int {
        guard generation == writing.load(ordering: .relaxed) else { return 0 }
        let w = written.load(ordering: .relaxed)
        return w - max(read.load(ordering: .acquiring), startAt.load(ordering: .relaxed))
    }

    /// Append interleaved left/right samples rendered for `generation`;
    /// what does not fit is dropped.
    func write(_ interleaved: [Int16], generation: Int = 0) {
        let w = written.load(ordering: .relaxed)
        if generation != writing.load(ordering: .relaxed) {
            writing.store(generation, ordering: .relaxed)
            startAt.store(w, ordering: .relaxed)
            startGeneration.store(generation, ordering: .releasing)
        }
        let n = min(interleaved.count / 2, capacity - (w - read.load(ordering: .acquiring)))
        guard n > 0 else { return }
        interleaved.withUnsafeBufferPointer { src in
            for i in 0..<n {
                let at = (w + i) % capacity
                buffer[at * 2] = src[i * 2]
                buffer[at * 2 + 1] = src[i * 2 + 1]
            }
        }
        written.store(w + n, ordering: .releasing)
    }

    /// Read up to `frames` into two float channels, filling the rest with
    /// silence. Returns the frames that were real.
    @discardableResult
    func read(into left: UnsafeMutablePointer<Float>, _ right: UnsafeMutablePointer<Float>, frames: Int) -> Int {
        var r = read.load(ordering: .relaxed)
        var playing = true
        let want = requested.load(ordering: .acquiring)
        if want != serving.load(ordering: .relaxed) {
            if startGeneration.load(ordering: .acquiring) == want {
                // The new player's first samples: skip what came before.
                r = max(r, startAt.load(ordering: .relaxed))
                serving.store(want, ordering: .relaxed)
            } else {
                // Cleared, and nothing of the new player yet: drop the old.
                r = written.load(ordering: .acquiring)
                playing = false
            }
        }
        let n = playing ? min(frames, written.load(ordering: .acquiring) - r) : 0
        for i in 0..<n {
            let at = (r + i) % capacity
            left[i] = Float(buffer[at * 2]) / 32768
            right[i] = Float(buffer[at * 2 + 1]) / 32768
        }
        for i in max(n, 0)..<frames {
            left[i] = 0
            right[i] = 0
        }
        read.store(r + max(n, 0), ordering: .releasing)
        return max(n, 0)
    }

    /// Ask the consumer to drop everything written so far; returns the
    /// generation to render the next player for. Main actor only.
    @discardableResult
    func clear() -> Int {
        requested.add(1, ordering: .releasing).newValue
    }
}

/// What plays: a player rendered ahead into a ring, and the ring played
/// through AVAudioEngine at the DSP's own 32 kHz, the engine resampling to
/// the output device.
@MainActor
final class ApuAudio {
    nonisolated static let sampleRate = 32_000.0
    /// How far ahead the ring is kept filled: 100 ms.
    nonisolated static let lead = 3200

    /// Off in tests: the ring fills and is read by hand, and nothing is
    /// played out loud.
    static var deviceEnabled = true

    let ring = SampleRing()
    private var engine: AVAudioEngine?
    private let queue = DispatchQueue(label: "romlens.apu.render", qos: .userInteractive)
    private var timer: DispatchSourceTimer?
    /// The player the queue renders, with the ring generation it renders
    /// for; swapped on the main actor, read on the queue under the lock.
    private let current = Mutex<(player: ApuPlayer, generation: Int)?>(nil)
    /// Held for the whole of a fill, so the queue and a test filling by
    /// hand are never two producers at once. Never taken on the audio
    /// thread.
    private let producing = Mutex<Void>(())
    /// Why the output could not start, if it could not.
    private(set) var problem: String?
    /// The output's level, 0–1.
    var volume: Float = 1 {
        didSet { engine?.mainMixerNode.outputVolume = volume }
    }
    /// The device is pulling samples.
    var isEngineRunning: Bool { engine?.isRunning ?? false }

    /// Play `player` from now, or stop with nil.
    func play(_ player: ApuPlayer?) {
        // The clear first: a fill under way for the old player writes for
        // the old generation, which the audio thread skips.
        let generation = ring.clear()
        current.withLock { $0 = player.map { ($0, generation) } }
        guard player != nil else {
            stopTimer()
            engine?.pause()
            return
        }
        fill()
        startTimer()
        startEngine()
    }

    var isRunning: Bool { current.withLock { $0 != nil } }

    /// Render until the ring holds `lead` frames, on the calling thread.
    /// The render queue calls it; tests call it to play without a device.
    nonisolated func fill() {
        producing.withLock { _ in
            guard let (player, generation) = current.withLock({ $0 }) else { return }
            let need = Self.lead - ring.pending(generation: generation)
            guard need > 0 else { return }
            let samples = player.render(samples: UInt32(min(need, ring.space)))
            // Swapped while rendering: these belong to a player no longer heard.
            guard current.withLock({ $0?.generation }) == generation else { return }
            ring.write(samples, generation: generation)
        }
    }

    private func startTimer() {
        guard timer == nil else { return }
        timer = Self.makeTimer(queue: queue, fill: { [weak self] in self?.fill() })
    }

    /// Built outside the main actor, so the handler carries no isolation
    /// the queue would trip over.
    private nonisolated static func makeTimer(queue: DispatchQueue, fill: @escaping @Sendable () -> Void) -> DispatchSourceTimer {
        let t = DispatchSource.makeTimerSource(queue: queue)
        t.schedule(deadline: .now(), repeating: .milliseconds(10), leeway: .milliseconds(2))
        t.setEventHandler(handler: fill)
        t.resume()
        return t
    }

    /// The node the engine pulls from, built outside the main actor: its
    /// block runs on the audio thread and only reads the ring.
    private nonisolated static func sourceNode(format: AVAudioFormat, ring: SampleRing) -> AVAudioSourceNode {
        AVAudioSourceNode(format: format) { _, _, frameCount, buffers in
            let list = UnsafeMutableAudioBufferListPointer(buffers)
            guard list.count >= 2,
                  let l = list[0].mData?.assumingMemoryBound(to: Float.self),
                  let r = list[1].mData?.assumingMemoryBound(to: Float.self)
            else { return noErr }
            ring.read(into: l, r, frames: Int(frameCount))
            return noErr
        }
    }

    private func stopTimer() {
        timer?.cancel()
        timer = nil
    }

    private func startEngine() {
        guard Self.deviceEnabled else { return }
        if let engine {
            if !engine.isRunning { try? engine.start() }
            return
        }
        let engine = AVAudioEngine()
        guard let format = AVAudioFormat(standardFormatWithSampleRate: Self.sampleRate, channels: 2) else { return }
        let source = Self.sourceNode(format: format, ring: ring)
        engine.attach(source)
        do {
            try engine.connectNode(source, to: engine.mainMixerNode, format: format)
            engine.mainMixerNode.outputVolume = volume
            try engine.start()
            self.engine = engine
            problem = nil
        } catch {
            problem = "The sound output could not start: \(error.localizedDescription)"
        }
    }

    func shutDown() {
        play(nil)
        engine?.stop()
        engine = nil
    }

    /// A timer left running would keep firing on the queue.
    isolated deinit {
        timer?.cancel()
        engine?.stop()
    }
}
