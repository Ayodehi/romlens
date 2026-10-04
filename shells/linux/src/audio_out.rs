//! What plays (docs/23, A12): a player rendered ahead into a lock-free ring on
//! a background thread, and the ring played through the system's sound server
//! (ALSA, which PipeWire and PulseAudio serve) by `cpal`. The DSP runs at 32
//! kHz and the device at whatever it likes, so the audio callback resamples.
//! Nothing on the audio thread allocates, locks or calls into the core. The
//! macOS twin is `ApuAudio`.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use romlens_ffi::ApuPlayer;

pub const SAMPLE_RATE: u32 = 32_000;
/// How far ahead the ring is kept filled: 100 ms.
pub const LEAD: usize = 3200;
/// Frames the ring holds: 0.25 s at 32 kHz.
pub const CAPACITY: usize = 8192;

/// Something that renders interleaved stereo 16-bit samples at 32 kHz.
pub trait Source: Send + Sync {
    fn render(&self, samples: u32) -> Vec<i16>;
}

impl Source for ApuPlayer {
    fn render(&self, samples: u32) -> Vec<i16> {
        ApuPlayer::render(self, samples)
    }
}

/// Stereo 16-bit samples from one producer to one consumer, without locks:
/// the producer renders on a background thread, the audio thread reads. A
/// frame is its left and right samples packed in one atomic word, so a reader
/// never sees half of one.
///
/// Only the consumer moves `read`. A clear is a request: the model's thread
/// bumps `requested`, and the producer tags what it writes with the request
/// its player was started under, marking where that player's samples begin.
/// From the request the consumer plays silence until the mark, then skips to
/// it, so what an old player rendered is never heard.
pub struct SampleRing {
    buffer: Vec<AtomicU32>,
    written: AtomicUsize,
    read: AtomicUsize,
    /// The latest clear asked for (the model's thread).
    requested: AtomicUsize,
    /// Where the samples of generation `start_generation` begin (producer).
    start_at: AtomicUsize,
    start_generation: AtomicUsize,
    /// The generation the producer last wrote (producer only).
    writing: AtomicUsize,
    /// The generation the consumer plays (consumer only).
    serving: AtomicUsize,
}

fn pack(left: i16, right: i16) -> u32 {
    u32::from(left as u16) << 16 | u32::from(right as u16)
}

fn unpack(v: u32) -> (i16, i16) {
    ((v >> 16) as u16 as i16, v as u16 as i16)
}

impl SampleRing {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: (0..capacity).map(|_| AtomicU32::new(0)).collect(),
            written: AtomicUsize::new(0),
            read: AtomicUsize::new(0),
            requested: AtomicUsize::new(0),
            start_at: AtomicUsize::new(0),
            start_generation: AtomicUsize::new(0),
            writing: AtomicUsize::new(0),
            serving: AtomicUsize::new(0),
        }
    }

    pub fn capacity(&self) -> usize {
        self.buffer.len()
    }

    /// Frames waiting to be read.
    pub fn available(&self) -> usize {
        self.written
            .load(Ordering::Acquire)
            .wrapping_sub(self.read.load(Ordering::Acquire))
    }

    pub fn space(&self) -> usize {
        self.capacity() - self.available()
    }

    /// Frames of `generation` waiting to be read, as the producer sees them:
    /// an older player's left in the ring do not count.
    pub fn pending(&self, generation: usize) -> usize {
        if generation != self.writing.load(Ordering::Relaxed) {
            return 0;
        }
        let from = self
            .read
            .load(Ordering::Acquire)
            .max(self.start_at.load(Ordering::Relaxed));
        self.written.load(Ordering::Relaxed).saturating_sub(from)
    }

    /// Append interleaved left and right samples, rendered before any clear;
    /// what does not fit is dropped. Returns the frames written.
    #[cfg(test)]
    pub fn write(&self, interleaved: &[i16]) -> usize {
        self.write_for(interleaved, 0)
    }

    /// Append samples rendered for `generation` (the value a clear returned).
    pub fn write_for(&self, interleaved: &[i16], generation: usize) -> usize {
        let w = self.written.load(Ordering::Relaxed);
        if generation != self.writing.load(Ordering::Relaxed) {
            self.writing.store(generation, Ordering::Relaxed);
            self.start_at.store(w, Ordering::Relaxed);
            self.start_generation.store(generation, Ordering::Release);
        }
        let n = (interleaved.len() / 2).min(self.space());
        for i in 0..n {
            self.buffer[(w + i) % self.capacity()].store(
                pack(interleaved[i * 2], interleaved[i * 2 + 1]),
                Ordering::Relaxed,
            );
        }
        self.written.store(w + n, Ordering::Release);
        n
    }

    /// The next frame, if there is one. The consumer's alone: it honours a
    /// clear here, by skipping to the new player's first sample, or by
    /// dropping everything until the new player has written one.
    pub fn pop(&self) -> Option<(i16, i16)> {
        let mut r = self.read.load(Ordering::Relaxed);
        let want = self.requested.load(Ordering::Acquire);
        if want != self.serving.load(Ordering::Relaxed) {
            if self.start_generation.load(Ordering::Acquire) == want {
                r = r.max(self.start_at.load(Ordering::Relaxed));
                self.serving.store(want, Ordering::Relaxed);
            } else {
                self.read
                    .store(self.written.load(Ordering::Acquire), Ordering::Release);
                return None;
            }
        }
        if self.written.load(Ordering::Acquire) == r {
            self.read.store(r, Ordering::Release);
            return None;
        }
        let v = self.buffer[r % self.capacity()].load(Ordering::Relaxed);
        self.read.store(r + 1, Ordering::Release);
        Some(unpack(v))
    }

    /// Ask the consumer to drop everything written so far; returns the
    /// generation to render the next player for. Never moves `read`.
    pub fn clear(&self) -> usize {
        self.requested.fetch_add(1, Ordering::AcqRel) + 1
    }
}

/// Linear resampling of the 32 kHz ring to the device's rate, one output frame
/// at a time. Allocation free, so it can run on the audio thread.
pub struct Resampler {
    step: f64,
    frac: f64,
    a: (f32, f32),
    b: (f32, f32),
    primed: bool,
}

fn to_f32((l, r): (i16, i16)) -> (f32, f32) {
    (f32::from(l) / 32768.0, f32::from(r) / 32768.0)
}

impl Resampler {
    pub fn new(device_rate: u32) -> Self {
        Self {
            step: f64::from(SAMPLE_RATE) / f64::from(device_rate.max(1)),
            frac: 0.0,
            a: (0.0, 0.0),
            b: (0.0, 0.0),
            primed: false,
        }
    }

    /// The next output frame: interpolated between two ring frames, or
    /// silence when the ring runs dry.
    pub fn next(&mut self, ring: &SampleRing) -> (f32, f32) {
        if !self.primed {
            let (Some(a), Some(b)) = (ring.pop(), ring.pop()) else {
                return (0.0, 0.0);
            };
            self.a = to_f32(a);
            self.b = to_f32(b);
            self.primed = true;
        }
        let t = self.frac as f32;
        let out = (
            self.a.0 + (self.b.0 - self.a.0) * t,
            self.a.1 + (self.b.1 - self.a.1) * t,
        );
        self.frac += self.step;
        while self.frac >= 1.0 {
            match ring.pop() {
                Some(next) => {
                    self.a = self.b;
                    self.b = to_f32(next);
                    self.frac -= 1.0;
                }
                None => {
                    // Dry: hold the last two and wait, so a gap is a pause
                    // rather than a squeal.
                    self.frac = 1.0 - f64::EPSILON;
                    self.primed = false;
                    break;
                }
            }
        }
        out
    }
}

/// Shared between the model's thread, the render thread and the callback.
struct Shared {
    ring: SampleRing,
    /// The player rendered, with the ring generation it renders for.
    current: Mutex<Option<(Arc<dyn Source>, usize)>>,
    /// Held for the whole of a fill, so the render thread and a test filling
    /// by hand are never two producers at once. The callback only reads, and
    /// never takes it.
    producer: Mutex<()>,
    /// The output's level, 0 to 1, as `f32` bits.
    volume: AtomicU32,
    stop: AtomicBool,
}

impl Shared {
    /// Render until the ring holds `LEAD` frames, on the calling thread.
    fn fill(&self) {
        let _producer = self.producer.lock().unwrap_or_else(|e| e.into_inner());
        let current = || self.current.lock().unwrap_or_else(|e| e.into_inner());
        let Some((player, generation)) = current().clone() else {
            return;
        };
        let need = LEAD.saturating_sub(self.ring.pending(generation));
        if need == 0 {
            return;
        }
        let samples = player.render(need.min(self.ring.space()) as u32);
        // Swapped while rendering: these belong to a player no longer heard.
        if current().as_ref().map(|(_, g)| *g) == Some(generation) {
            self.ring.write_for(&samples, generation);
        }
    }
}

pub struct ApuAudio {
    shared: Arc<Shared>,
    device_enabled: bool,
    thread: Option<JoinHandle<()>>,
    stream: Option<cpal::Stream>,
    /// Why the output could not start, if it could not.
    pub problem: Option<String>,
}

// The ring, fill and volume are what tests and a volume control reach for;
// the model plays and stops.
#[allow(dead_code)]
impl ApuAudio {
    /// `device_enabled` off, as in tests: the ring fills and is read by hand,
    /// and nothing is played out loud.
    pub fn new(device_enabled: bool) -> Self {
        Self {
            shared: Arc::new(Shared {
                ring: SampleRing::new(CAPACITY),
                current: Mutex::new(None),
                producer: Mutex::new(()),
                volume: AtomicU32::new(1.0f32.to_bits()),
                stop: AtomicBool::new(false),
            }),
            device_enabled,
            thread: None,
            stream: None,
            problem: None,
        }
    }

    pub fn ring(&self) -> &SampleRing {
        &self.shared.ring
    }

    pub fn set_volume(&self, v: f32) {
        self.shared
            .volume
            .store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.shared.volume.load(Ordering::Relaxed))
    }

    pub fn is_running(&self) -> bool {
        self.shared
            .current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    /// Render until the ring holds `LEAD` frames, on the calling thread. The
    /// render thread calls it; tests call it to play without a device.
    pub fn fill(&self) {
        self.shared.fill();
    }

    /// Play `player` from now, or stop with `None`.
    pub fn play(&mut self, player: Option<Arc<dyn Source>>) {
        let starting = player.is_some();
        // The clear first: a fill under way for the old player writes for the
        // old generation, which the callback skips.
        let generation = self.shared.ring.clear();
        *self
            .shared
            .current
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = player.map(|p| (p, generation));
        if !starting {
            if let Some(s) = &self.stream {
                let _ = s.pause();
            }
            return;
        }
        self.shared.fill();
        self.start_thread();
        self.start_stream();
    }

    fn start_thread(&mut self) {
        if self.thread.is_some() {
            return;
        }
        let shared = Arc::clone(&self.shared);
        self.thread = std::thread::Builder::new()
            .name("romlens-apu-render".into())
            .spawn(move || {
                while !shared.stop.load(Ordering::Relaxed) {
                    shared.fill();
                    std::thread::sleep(Duration::from_millis(10));
                }
            })
            .ok();
    }

    fn start_stream(&mut self) {
        if !self.device_enabled {
            return;
        }
        if let Some(s) = &self.stream {
            if let Err(e) = s.play() {
                self.problem = Some(format!("The sound output could not start: {e}"));
            }
            return;
        }
        match build_stream(Arc::clone(&self.shared)) {
            Ok(stream) => {
                self.problem = stream
                    .play()
                    .err()
                    .map(|e| format!("The sound output could not start: {e}"));
                self.stream = Some(stream);
            }
            Err(e) => self.problem = Some(format!("The sound output could not start: {e}")),
        }
    }

    pub fn shut_down(&mut self) {
        self.play(None);
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.stream = None;
    }
}

impl Drop for ApuAudio {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        *self
            .shared
            .current
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn build_stream(shared: Arc<Shared>) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "there is no sound output device".to_owned())?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    match format {
        SampleFormat::F32 => run::<f32>(&device, config, shared),
        SampleFormat::I16 => run::<i16>(&device, config, shared),
        SampleFormat::U16 => run::<u16>(&device, config, shared),
        SampleFormat::I32 => run::<i32>(&device, config, shared),
        other => Err(format!(
            "the output's sample format ({other}) is not supported"
        )),
    }
}

fn run<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels).max(1);
    let mut resampler = Resampler::new(config.sample_rate);
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                let volume = f32::from_bits(shared.volume.load(Ordering::Relaxed));
                for frame in data.chunks_mut(channels) {
                    let (l, r) = resampler.next(&shared.ring);
                    let (l, r) = (l * volume, r * volume);
                    for (i, out) in frame.iter_mut().enumerate() {
                        let v = match (channels, i) {
                            (1, _) => (l + r) / 2.0,
                            (_, 0) => l,
                            (_, 1) => r,
                            _ => 0.0,
                        };
                        *out = T::from_sample(v);
                    }
                }
            },
            |e| eprintln!("sound output: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plays a rising ramp, left = n, right = -n, and counts what it made.
    struct Ramp(AtomicUsize);

    impl Source for Ramp {
        fn render(&self, samples: u32) -> Vec<i16> {
            let start = self.0.fetch_add(samples as usize, Ordering::Relaxed);
            (0..samples as usize)
                .flat_map(|i| {
                    let n = (start + i) as i16;
                    [n, -n]
                })
                .collect()
        }
    }

    #[test]
    fn the_ring_keeps_frames_in_order_and_drops_what_does_not_fit() {
        let r = SampleRing::new(4);
        assert_eq!(r.write(&[1, -1, 2, -2, 3, -3]), 3);
        assert_eq!((r.available(), r.space()), (3, 1));
        assert_eq!(r.write(&[4, -4, 5, -5]), 1, "only one frame fits");
        assert_eq!(r.pop(), Some((1, -1)));
        assert_eq!(r.pop(), Some((2, -2)));
        // The slot freed is reused: indices run past the capacity.
        assert_eq!(r.write(&[6, -6, 7, -7]), 2);
        let rest: Vec<_> = std::iter::from_fn(|| r.pop()).collect();
        assert_eq!(rest, [(3, -3), (4, -4), (6, -6), (7, -7)]);
        assert_eq!(r.pop(), None);
    }

    /// A clear is honoured by the reader: what was written before it, and
    /// what the old player rendered after it, is never heard.
    #[test]
    fn a_clear_drops_what_the_old_player_wrote() {
        let r = SampleRing::new(16);
        r.write(&[100, 100, 100, 100]);
        let generation = r.clear();
        r.write(&[200, 200]);
        assert_eq!(r.pop(), None, "nothing of the new player yet");
        r.write(&[200, 200]);
        r.write_for(&[16384, 16384], generation);
        assert_eq!(r.pending(generation), 1);
        assert_eq!(r.pop(), Some((16384, 16384)));
        assert_eq!(r.pop(), None);
        assert_eq!(r.pending(generation), 0);
    }

    #[test]
    fn a_frame_survives_the_packing_whatever_its_sign() {
        for (l, r) in [(0, 0), (i16::MIN, i16::MAX), (-1, 1), (1234, -4321)] {
            assert_eq!(unpack(pack(l, r)), (l, r));
        }
    }

    #[test]
    fn playing_fills_the_ring_to_the_lead_and_stopping_empties_it() {
        let mut out = ApuAudio::new(false);
        out.play(Some(Arc::new(Ramp(AtomicUsize::new(0)))));
        assert!(out.is_running());
        assert_eq!(out.ring().available(), LEAD);
        // The render thread tops it up as the callback drains it.
        for _ in 0..1000 {
            out.ring().pop();
        }
        out.fill();
        assert_eq!(out.ring().available(), LEAD, "refilled to the lead");
        out.play(None);
        assert!(!out.is_running());
        // The callback honours the stop: nothing more is heard.
        assert_eq!(out.ring().pop(), None);
        assert_eq!(out.ring().available(), 0);
        out.shut_down();
    }

    #[test]
    fn the_resampler_runs_at_the_ratio_of_the_two_rates() {
        let ring = SampleRing::new(CAPACITY);
        let ramp: Vec<i16> = (0..4000i16).flat_map(|n| [n, n]).collect();
        ring.write(&ramp[..CAPACITY.min(4000) * 2]);
        // 64 kHz out of 32 kHz in: two output frames per ring frame.
        let mut up = Resampler::new(64_000);
        let before = ring.available();
        for _ in 0..1000 {
            up.next(&ring);
        }
        assert!((before - ring.available()).abs_diff(500) <= 2);
        // 16 kHz out: one ring frame is consumed every two outputs... the
        // other way round, two ring frames per output frame.
        let ring = SampleRing::new(CAPACITY);
        ring.write(&ramp[..4000 * 2]);
        let mut down = Resampler::new(16_000);
        let before = ring.available();
        for _ in 0..1000 {
            down.next(&ring);
        }
        assert!((before - ring.available()).abs_diff(2000) <= 3);
    }

    #[test]
    fn resampling_a_ramp_stays_a_ramp_and_a_dry_ring_is_silence() {
        let ring = SampleRing::new(CAPACITY);
        let ramp: Vec<i16> = (0..2000i16).flat_map(|n| [n * 8, n * 8]).collect();
        ring.write(&ramp);
        let mut rs = Resampler::new(48_000);
        let mut last = -1.0f32;
        for _ in 0..1500 {
            let (l, r) = rs.next(&ring);
            assert!(l >= last, "monotone: {l} after {last}");
            assert_eq!(l, r);
            last = l;
        }
        // Run it dry: silence, and no panic.
        let dry = SampleRing::new(8);
        let mut rs = Resampler::new(44_100);
        assert_eq!(rs.next(&dry), (0.0, 0.0));
    }

    #[test]
    fn volume_is_a_level_between_nothing_and_full() {
        let out = ApuAudio::new(false);
        out.set_volume(0.5);
        assert_eq!(out.volume(), 0.5);
        out.set_volume(7.0);
        assert_eq!(out.volume(), 1.0);
        out.set_volume(-1.0);
        assert_eq!(out.volume(), 0.0);
    }
}
