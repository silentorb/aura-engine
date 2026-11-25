//! Transport render-ahead: one render thread runs the graph into a ring buffer ahead of the
//! playhead, so the audio callback only copies frames.

use aura_render::{AudioSignal, Frame, Renderer, BLOCK_FRAMES};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

/// How long the render thread waits when the ring is full.
const RENDER_IDLE: Duration = Duration::from_millis(2);

/// Single-producer, single-consumer ring of stereo frames. Frames are packed into `u64`
/// atomics, so neither side locks or allocates.
pub(crate) struct Ring {
    frames: Box<[AtomicU64]>,
    /// Frames written and read since the ring was made; `written - read` are queued.
    written: AtomicU64,
    read: AtomicU64,
}

fn pack([left, right]: [f32; 2]) -> u64 {
    u64::from(left.to_bits()) | (u64::from(right.to_bits()) << 32)
}

fn unpack(bits: u64) -> [f32; 2] {
    [f32::from_bits(bits as u32), f32::from_bits((bits >> 32) as u32)]
}

impl Ring {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            frames: (0..capacity.max(BLOCK_FRAMES))
                .map(|_| AtomicU64::new(0))
                .collect(),
            written: AtomicU64::new(0),
            read: AtomicU64::new(0),
        }
    }

    fn capacity(&self) -> u64 {
        self.frames.len() as u64
    }

    /// Frames that fit before the ring is full.
    pub(crate) fn free(&self) -> u64 {
        let queued = self.written.load(Ordering::Relaxed) - self.read.load(Ordering::Acquire);
        self.capacity() - queued
    }

    /// Writer side: appends `frames` (which must fit, see [`Ring::free`]).
    pub(crate) fn push(&self, frames: &[[f32; 2]]) {
        let written = self.written.load(Ordering::Relaxed);
        for (offset, frame) in frames.iter().enumerate() {
            let slot = ((written + offset as u64) % self.capacity()) as usize;
            self.frames[slot].store(pack(*frame), Ordering::Relaxed);
        }
        self.written
            .store(written + frames.len() as u64, Ordering::Release);
    }

    /// Reader side: fills `out` with queued frames. Returns how many it filled; the rest of
    /// `out` is left as it was.
    pub(crate) fn pop_into(&self, out: &mut [[f32; 2]]) -> usize {
        let read = self.read.load(Ordering::Relaxed);
        let available = (self.written.load(Ordering::Acquire) - read) as usize;
        let count = available.min(out.len());
        for (offset, frame) in out[..count].iter_mut().enumerate() {
            let slot = ((read + offset as u64) % self.capacity()) as usize;
            *frame = unpack(self.frames[slot].load(Ordering::Relaxed));
        }
        self.read.store(read + count as u64, Ordering::Release);
        count
    }
}

/// A signal rendered ahead into a [`Ring`] by its own thread, from frame `start`.
pub(crate) struct RenderAhead {
    pub(crate) ring: Arc<Ring>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl RenderAhead {
    /// Fills the ring on this thread first (blocking while, for example, a plugin renders up to
    /// `start`), then keeps it full from a render thread.
    pub(crate) fn start(signal: AudioSignal, start: Frame, ahead_frames: usize) -> Self {
        let ring = Arc::new(Ring::new(ahead_frames));
        let mut renderer = Renderer::new(signal);
        let mut block = vec![[0.0; 2]; BLOCK_FRAMES];
        let mut next = start;
        fill(&ring, &mut renderer, &mut block, &mut next);

        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (ring, stop) = (Arc::clone(&ring), Arc::clone(&stop));
            std::thread::Builder::new()
                .name("aura-transport-render".into())
                .spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        if !fill(&ring, &mut renderer, &mut block, &mut next) {
                            std::thread::sleep(RENDER_IDLE);
                        }
                    }
                })
        };
        let thread = match thread {
            Ok(thread) => Some(thread),
            Err(error) => {
                // Without a render thread, playback runs out after the prefilled frames and
                // the rest reads as silence (counted as underruns).
                eprintln!("aura: could not start transport rendering: {error}");
                None
            }
        };
        Self { ring, stop, thread }
    }
}

/// Renders blocks into the ring while a whole block fits. Returns whether it rendered any.
fn fill(ring: &Ring, renderer: &mut Renderer, block: &mut [[f32; 2]], next: &mut Frame) -> bool {
    let mut rendered = false;
    while ring.free() >= block.len() as u64 {
        renderer.render_frames(*next, block);
        ring.push(block);
        *next += block.len() as Frame;
        rendered = true;
    }
    rendered
}

impl Drop for RenderAhead {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aura_render::{Frame, Signal};

    /// Each frame's value is its index.
    struct Ramp;

    impl Signal for Ramp {
        fn render(&mut self, start: Frame, out: &mut [f32]) {
            for (k, sample) in out.iter_mut().enumerate() {
                *sample = (start + k as Frame) as f32;
            }
        }
    }

    #[test]
    fn ring_hands_frames_over_in_order_and_wraps() {
        let ring = Ring::new(BLOCK_FRAMES);
        let mut out = vec![[0.0; 2]; 100];
        for round in 0..10 {
            let frames: Vec<[f32; 2]> = (0..100).map(|k| [(round * 100 + k) as f32, 0.5]).collect();
            ring.push(&frames);
            assert_eq!(ring.pop_into(&mut out), 100);
            assert_eq!(out, frames);
        }
        assert_eq!(ring.pop_into(&mut out), 0, "empty");
    }

    #[test]
    fn render_ahead_prefills_from_the_start_frame_and_keeps_up() {
        let ahead = RenderAhead::start(AudioSignal::Mono(Box::new(Ramp)), 1_000, 4 * BLOCK_FRAMES);
        let mut out = vec![[0.0; 2]; 3 * BLOCK_FRAMES];
        assert_eq!(ahead.ring.pop_into(&mut out), out.len(), "prefilled before playback");
        assert_eq!(out[0], [1_000.0; 2]);
        let mut next = 1_000.0 + out.len() as f32;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while next < 20_000.0 {
            assert!(std::time::Instant::now() < deadline, "render thread fell behind");
            let count = ahead.ring.pop_into(&mut out);
            for frame in &out[..count] {
                assert_eq!(*frame, [next; 2]);
                next += 1.0;
            }
        }
    }
}
