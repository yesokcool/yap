//! A synthetic "screen" for tests and screenshots: colour bars, a square
//! that moves across, a frame counter in big blocky digits and, with sound
//! on, a two-tone chord (440 Hz left, 660 Hz right). Never the real screen.

use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

pub const ID: &str = "pattern";
/// The pattern's size (16:9; the receiving side checks it arrives like this).
pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 720;

pub fn source() -> Source {
    Source { id: ID.into(), name: format!("Test pattern ({WIDTH} × {HEIGHT})"), kind: SourceKind::Screen, width: WIDTH, height: HEIGHT }
}

struct Pattern {
    stop: Arc<AtomicBool>,
    fps: Arc<AtomicU32>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Capture for Pattern {
    fn set_options(&mut self, options: Options) {
        self.fps.store(options.fps.clamp(1, 60), Ordering::Relaxed);
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for Pattern {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn start(options: Options, mut on_frame: OnFrame, on_audio: Option<OnAudio>) -> Box<dyn Capture> {
    let stop = Arc::new(AtomicBool::new(false));
    let fps = Arc::new(AtomicU32::new(options.fps.clamp(1, 60)));
    let mut threads = Vec::new();
    let (s, f) = (stop.clone(), fps.clone());
    threads.push(
        std::thread::Builder::new()
            .name("yap-pattern".into())
            .spawn(move || {
                let mut n: u64 = 0;
                let mut next = Instant::now();
                while !s.load(Ordering::Relaxed) {
                    on_frame(draw(WIDTH, HEIGHT, n));
                    n += 1;
                    next += Duration::from_secs_f64(1.0 / f.load(Ordering::Relaxed) as f64);
                    let now = Instant::now();
                    if next > now {
                        std::thread::sleep(next - now);
                    } else {
                        next = now;
                    }
                }
            })
            .expect("pattern thread"),
    );
    if let Some(mut on_audio) = on_audio.filter(|_| options.sound) {
        let s = stop.clone();
        threads.push(
            std::thread::Builder::new()
                .name("yap-pattern-sound".into())
                .spawn(move || {
                    // 10 ms chunks, kept on the wall clock.
                    let mut t: u64 = 0;
                    let start = Instant::now();
                    let mut buf = vec![0f32; 960];
                    while !s.load(Ordering::Relaxed) {
                        for i in 0..480 {
                            let x = (t + i as u64) as f32 / 48_000.0;
                            let wobble = 0.6 + 0.4 * (std::f32::consts::TAU * 0.5 * x).sin();
                            buf[2 * i] = 0.2 * wobble * (std::f32::consts::TAU * 440.0 * x).sin();
                            buf[2 * i + 1] = 0.2 * wobble * (std::f32::consts::TAU * 660.0 * x).sin();
                        }
                        on_audio(&buf);
                        t += 480;
                        let due = start + Duration::from_micros(t * 1_000_000 / 48_000);
                        let now = Instant::now();
                        if due > now {
                            std::thread::sleep(due - now);
                        }
                    }
                })
                .expect("pattern sound thread"),
        );
    }
    Box::new(Pattern { stop, fps, threads })
}

/// One frame of the pattern (frame number `n`).
pub fn draw(w: u32, h: u32, n: u64) -> Frame {
    let (wu, hu) = (w as usize, h as usize);
    let stride = wu * 4;
    let mut data = vec![0u8; stride * hu];
    // Top half: eight colour bars (B, G, R order in memory).
    const BARS: [[u8; 3]; 8] = [[255, 255, 255], [0, 255, 255], [255, 255, 0], [0, 255, 0], [255, 0, 255], [0, 0, 255], [255, 0, 0], [40, 40, 40]];
    for y in 0..hu {
        let row = &mut data[y * stride..(y + 1) * stride];
        for x in 0..wu {
            let c = if y < hu / 2 { BARS[x * 8 / wu] } else { [0x33, 0x22, 0x11] };
            row[x * 4..x * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    let mut fill = |x0: usize, y0: usize, ww: usize, hh: usize, c: [u8; 3]| {
        for y in y0.min(hu)..(y0 + hh).min(hu) {
            for x in x0.min(wu)..(x0 + ww).min(wu) {
                data[y * stride + x * 4..y * stride + x * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
    };
    // A white square that crosses the lower half (it moves every frame).
    let side = hu / 6;
    let x = (n as usize * 8) % (wu - side);
    fill(x, hu * 9 / 16, side, side, [255, 255, 255]);
    // The frame number, as blocky seven-segment digits.
    let digits = n.to_string();
    let (dw, dh, t) = (hu / 18, hu / 9, (hu / 90).max(2));
    for (i, ch) in digits.chars().enumerate() {
        let d = ch.to_digit(10).unwrap_or(0) as usize;
        const SEG: [u8; 10] = [0b1110111, 0b0010010, 0b1011101, 0b1011011, 0b0111010, 0b1101011, 0b1101111, 0b1010010, 0b1111111, 0b1111011];
        let (ox, oy) = (hu / 24 + i * (dw + dw / 2), hu - dh - hu / 24);
        let s = SEG[d];
        let c = [0, 220, 255];
        if s & 0b1000000 != 0 {
            fill(ox, oy, dw, t, c);
        }
        if s & 0b0100000 != 0 {
            fill(ox, oy, t, dh / 2, c);
        }
        if s & 0b0010000 != 0 {
            fill(ox + dw - t, oy, t, dh / 2, c);
        }
        if s & 0b0001000 != 0 {
            fill(ox, oy + dh / 2 - t / 2, dw, t, c);
        }
        if s & 0b0000100 != 0 {
            fill(ox, oy + dh / 2, t, dh / 2, c);
        }
        if s & 0b0000010 != 0 {
            fill(ox + dw - t, oy + dh / 2, t, dh / 2, c);
        }
        if s & 0b0000001 != 0 {
            fill(ox, oy + dh - t, dw, t, c);
        }
    }
    Frame { width: w, height: h, stride, data }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_move() {
        let a = draw(320, 180, 0);
        let b = draw(320, 180, 1);
        assert_eq!((a.width, a.height, a.stride, a.data.len()), (320, 180, 1280, 320 * 180 * 4));
        assert_ne!(a.data, b.data, "the square moves (and the counter changes)");
        // The first bar is white, the last nearly black.
        assert_eq!(&a.data[0..3], &[255, 255, 255]);
        assert_eq!(&a.data[(319 * 4)..(319 * 4 + 3)], &[40, 40, 40]);
    }
}
