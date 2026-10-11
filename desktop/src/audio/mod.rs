//! Audio: capture -> 80 Hz high-pass -> echo cancellation (`echo.rs`, fed
//! with what the speakers play) -> RNNoise -> automatic gain -> the
//! transmit gate (`gate.rs`: voice activity / push-to-talk with fades, a
//! release tail and a pre-roll) -> mute -> 20 ms frames (fanned out to every
//! peer's Opus encoder), and the playout mixer that pulls every peer's jitter
//! buffer from the output device's clock.
//!
//! Threads: the cpal callbacks (or a WAV/null pacer in headless mode), one
//! capture-processing thread. Nothing here touches the UI or the network.

pub mod clips;
pub mod decode;
pub mod dsp;
pub mod echo;
#[cfg(test)]
mod echo_sim;
pub mod gate;
pub mod jitter;
pub mod resample;
pub mod space_sound;
pub mod wav;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::{Mutex, RwLock};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

use crate::config::InputMode;
use jitter::JitterBuffer;
use resample::Resampler;

pub const RATE: u32 = 48_000;
/// 20 ms Opus frames.
pub const FRAME: usize = 960;
/// 10 ms analysis blocks (RNNoise's frame size).
const BLOCK: usize = 480;
/// Capture this late means processing is behind: noise removal is skipped
/// until it catches up (fail open: the plain mic beats dropped audio).
const BEHIND: Duration = Duration::from_millis(100);
/// Remote speaking threshold, linear RMS (~ -34 dBFS), same as the web client.
pub const REMOTE_SPEAKING_LEVEL: f32 = 0.02;

/// Atomic f32 helper.
#[derive(Default)]
pub struct AtomicF32(AtomicU32);
impl AtomicF32 {
    pub fn new(v: f32) -> Self {
        Self(AtomicU32::new(v.to_bits()))
    }
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
    pub fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed)
    }
}

/// Transmit gate settings + local meters, shared with the UI, the PTT
/// poller and the voice engine (the gate itself runs on the capture thread).
pub struct Gate {
    pub muted: AtomicBool,
    pub deafened: AtomicBool,
    pub ptt_down: AtomicBool,
    mode: AtomicU8,
    pub vad_threshold: AtomicF32,
    pub noise_suppression: AtomicBool,
    /// Automatic gain (V12): speech towards −23 dBFS, at most +20 dB.
    pub auto_gain: AtomicBool,
    /// Echo cancellation (AEC3 on the mic, against what the speakers play).
    pub echo_cancellation: AtomicBool,
    /// What the echo canceller is doing (running, its delay estimate), for
    /// the logs and the call report; updated every 100 ms.
    pub echo: Mutex<echo::EchoStats>,
    /// A mic test with nobody listening: push-to-talk acts like always-on.
    pub test: AtomicBool,
    pub mic_ok: AtomicBool,
    /// Local mic level (dBFS, after noise suppression and gain, before the gate).
    pub level_db: AtomicF32,
    /// Where voice activity opens right now (your sensitivity, or higher in a noisy room).
    pub open_at: AtomicF32,
    pub transmitting: AtomicBool,
    pub speaking: AtomicBool,
    /// The mic has delivered nothing but digital silence for 5 s (a mute
    /// switch, a dead device): "Your mic is sending silence".
    pub silent: AtomicBool,
    /// The automatic gain now (dB), for the logs.
    pub agc_db: AtomicF32,
    /// Capture is behind: noise removal skipped for now.
    pub behind: AtomicBool,
}

impl Gate {
    fn new() -> Self {
        Self {
            muted: AtomicBool::new(false),
            deafened: AtomicBool::new(false),
            ptt_down: AtomicBool::new(false),
            mode: AtomicU8::new(0),
            vad_threshold: AtomicF32::new(-50.0),
            noise_suppression: AtomicBool::new(true),
            auto_gain: AtomicBool::new(true),
            echo_cancellation: AtomicBool::new(true),
            echo: Mutex::new(echo::EchoStats::default()),
            test: AtomicBool::new(false),
            mic_ok: AtomicBool::new(false),
            level_db: AtomicF32::new(-100.0),
            open_at: AtomicF32::new(-50.0),
            transmitting: AtomicBool::new(false),
            speaking: AtomicBool::new(false),
            silent: AtomicBool::new(false),
            agc_db: AtomicF32::new(0.0),
            behind: AtomicBool::new(false),
        }
    }
    pub fn mode(&self) -> InputMode {
        match self.mode.load(Ordering::Relaxed) {
            1 => InputMode::Ptt,
            2 => InputMode::Open,
            _ => InputMode::Vad,
        }
    }
    fn tx_mode(&self) -> gate::Mode {
        match self.mode() {
            InputMode::Vad => gate::Mode::Vad,
            InputMode::Ptt => gate::Mode::Ptt,
            InputMode::Open => gate::Mode::Open,
        }
    }
    pub fn set_mode(&self, m: InputMode) {
        self.mode.store(
            match m {
                InputMode::Vad => 0,
                InputMode::Ptt => 1,
                InputMode::Open => 2,
            },
            Ordering::Relaxed,
        );
    }
}

/// One 20 ms frame of (gated) mono mic audio, ready for every peer's encoder.
pub struct AudioFrame {
    pub pcm: [i16; FRAME],
    /// RFC 6464 level: 0 (loud) .. 127 (silence), in -dBov.
    pub level_dbov: u8,
    pub voice: bool,
    pub at: Instant,
}

/// A remote peer's audio as it reaches the mixer.
pub struct PeerStream {
    /// Whose (it changes when they resume the call under a new session).
    sid: RwLock<String>,
    jb: Mutex<JitterBuffer>,
    pub volume: AtomicF32,
    /// Past 100 % volume, a limiter keeps a boosted voice under full scale (V13).
    limiter: Mutex<dsp::Limiter>,
    /// RMS of the last decoded frame (pre-volume).
    pub level: AtomicF32,
    last_loud_ms: AtomicU64,
    epoch: Instant,
}

impl PeerStream {
    pub fn sid(&self) -> String {
        self.sid.read().clone()
    }
    pub fn push_rtp(&self, seq: u16, ts: u32, payload: &[u8]) {
        let at = self.epoch.elapsed().as_secs_f64() * 1000.0;
        self.jb.lock().push(seq, ts, payload, at);
    }
    /// A RED packet (the frame before rides along): see `JitterBuffer::push_red`.
    pub fn push_red(&self, seq: u16, ts: u32, payload: &[u8]) {
        let at = self.epoch.elapsed().as_secs_f64() * 1000.0;
        self.jb.lock().push_red(seq, ts, payload, at);
    }
    pub fn stats(&self) -> jitter::JitterStats {
        self.jb.lock().stats
    }
    /// Speaking = loud within the last 250 ms.
    pub fn speaking(&self) -> bool {
        let last = self.last_loud_ms.load(Ordering::Relaxed);
        last > 0 && (self.epoch.elapsed().as_millis() as u64).saturating_sub(last) < 250
    }
}

pub struct Mixer {
    streams: RwLock<Vec<Arc<PeerStream>>>,
    gate: Arc<Gate>,
    tones: Mutex<VecDeque<f32>>,
    /// Soundboard, chimes, previews, instruments (limited; silent while deafened).
    pub clips: Mutex<clips::Clips>,
    epoch: Instant,
    /// Total frames rendered (for the headless stats).
    pub rendered: AtomicU64,
    /// Mixed output RMS of the last block.
    pub out_level: AtomicF32,
}

impl Mixer {
    /// Render `out.len() / 2` stereo frames at 48 kHz.
    pub fn render(&self, out: &mut [f32], scratch: &mut Vec<f32>) {
        out.fill(0.0);
        scratch.resize(out.len(), 0.0);
        let deaf = self.gate.deafened.load(Ordering::Relaxed);
        let now_ms = self.epoch.elapsed().as_millis() as u64;
        for s in self.streams.read().iter() {
            s.jb.lock().pull(scratch);
            let rms = (scratch.iter().map(|x| x * x).sum::<f32>() / scratch.len().max(1) as f32).sqrt();
            s.level.set(rms);
            if rms > REMOTE_SPEAKING_LEVEL && !deaf {
                s.last_loud_ms.store(now_ms.max(1), Ordering::Relaxed);
            }
            if deaf {
                continue;
            }
            let v = s.volume.get();
            if v > 1.001 {
                s.limiter.lock().process(scratch, v);
                for (o, x) in out.iter_mut().zip(scratch.iter()) {
                    *o += x;
                }
            } else {
                for (o, x) in out.iter_mut().zip(scratch.iter()) {
                    *o += x * v;
                }
            }
        }
        {
            let mut tones = self.tones.lock();
            for o in out.iter_mut() {
                match tones.pop_front() {
                    Some(t) => *o += t,
                    None => break,
                }
            }
        }
        {
            let mut clips = self.clips.lock();
            if deaf {
                clips.clear();
            } else {
                clips.render(out);
            }
        }
        let mut sum = 0f32;
        for o in out.iter_mut() {
            // Soft clip: transparent below ~0.8, never wraps.
            if o.abs() > 0.8 {
                *o = o.signum() * (0.8 + 0.2 * ((o.abs() - 0.8) / 0.2).tanh());
            }
            sum += *o * *o;
        }
        self.out_level.set((sum / out.len().max(1) as f32).sqrt());
        self.rendered.fetch_add(out.len() as u64 / 2, Ordering::Relaxed);
    }

    /// Queue a little synthesized blip (join/leave/mute), like the web client.
    pub fn play_notes(&self, notes: &[f32], step: f32, length: f32, volume: f32, triangle: bool) {
        let tone = Tone { notes, step, length, volume, wave: if triangle { Wave::Triangle } else { Wave::Sine } };
        let buf = synth(&tone, RATE);
        let mut tones = self.tones.lock();
        tones.clear();
        for s in buf {
            tones.push_back(s);
            tones.push_back(s);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Wave {
    Sine,
    Triangle,
    Square,
}

/// A few notes like web/js/sounds.js: attack 12 ms, exponential decay.
pub struct Tone<'a> {
    pub notes: &'a [f32],
    pub step: f32,
    pub length: f32,
    pub volume: f32,
    pub wave: Wave,
}

/// Mono samples at `rate`.
pub fn synth(t: &Tone<'_>, rate: u32) -> Vec<f32> {
    let rate = rate as f32;
    let total = ((t.notes.len() as f32 - 1.0).max(0.0) * t.step + t.length + 0.03) * rate;
    let mut buf = vec![0f32; total as usize];
    for (i, &f) in t.notes.iter().enumerate() {
        let start = ((0.01 + i as f32 * t.step) * rate) as usize;
        for n in 0..(t.length * rate) as usize {
            let x = n as f32 / rate;
            let env = if x < 0.012 { x / 0.012 } else { 0.0001f32.powf((x - 0.012) / (t.length - 0.012)) };
            let phase = f * x;
            let wave = match t.wave {
                Wave::Sine => (2.0 * std::f32::consts::PI * phase).sin(),
                Wave::Triangle => 4.0 * (phase - (phase + 0.5).floor()).abs() - 1.0,
                // Softened square (band-limited-ish via tanh) so it chimes rather than buzzes.
                Wave::Square => ((2.0 * std::f32::consts::PI * phase).sin() * 4.0).tanh(),
            };
            if let Some(slot) = buf.get_mut(start + n) {
                *slot += wave * env * t.volume;
            }
        }
    }
    buf
}

impl AudioEngine {
    /// A notification chime: through the call's mixer when we're in voice,
    /// otherwise on a short-lived output stream of its own.
    pub fn chime(&self, tone: &Tone<'_>, device: &str) {
        if self.output_running() {
            let buf = synth(tone, RATE);
            let mut tones = self.mixer.tones.lock();
            tones.clear();
            for s in buf {
                tones.push_back(s);
                tones.push_back(s);
            }
            return;
        }
        let tone = Tone { notes: tone.notes, ..*tone };
        let notes: Vec<f32> = tone.notes.to_vec();
        let (step, length, volume, wave) = (tone.step, tone.length, tone.volume, tone.wave);
        let device = device.to_string();
        let _ = std::thread::Builder::new().name("yap-chime".into()).spawn(move || {
            let tone = Tone { notes: &notes, step, length, volume, wave };
            if let Err(e) = play_detached(&tone, &device) {
                log::debug!("chime: {e}");
            }
        });
    }
}

impl AudioEngine {
    /// Play a clip (mono, 48 kHz) at `gain`: over the call when we're in one,
    /// otherwise on a short-lived output stream of its own (`device`).
    pub fn play_clip(&self, samples: Arc<Vec<f32>>, gain: f32, bus: clips::Bus, device: &str) {
        if self.output_running() {
            self.mixer.clips.lock().push(samples, gain, bus);
            return;
        }
        self.sfx.clips.lock().push(samples, gain, bus);
        self.ensure_sfx(device);
    }

    /// Stop every clip (the soundboard was just muted).
    pub fn stop_clips(&self) {
        self.mixer.clips.lock().clear();
        self.sfx.clips.lock().clear();
    }

    fn ensure_sfx(&self, device: &str) {
        if self.sfx_alive.swap(true, Ordering::SeqCst) {
            return;
        }
        let (mixer, alive, device) = (self.sfx.clone(), self.sfx_alive.clone(), device.to_string());
        let spawned = std::thread::Builder::new().name("yap-sfx".into()).spawn(move || {
            loop {
                let (stop_tx, stop_rx) = std_mpsc::channel::<()>();
                let (ready_tx, ready_rx) = std_mpsc::channel::<Result<Option<String>, String>>();
                let m = mixer.clone();
                let dev = device.clone();
                let thread = std::thread::Builder::new().name("yap-sfx-out".into()).spawn(move || run_device_output(&dev, m, None, stop_rx, ready_tx, None, None));
                let ok = thread.is_ok() && matches!(ready_rx.recv_timeout(Duration::from_secs(5)), Ok(Ok(_)));
                if !ok {
                    log::debug!("sound effects: no output device");
                    mixer.clips.lock().clear();
                }
                // Keep the stream warm while things play (and a little after: piano runs).
                let mut idle_since: Option<Instant> = None;
                if ok {
                    loop {
                        std::thread::sleep(Duration::from_millis(250));
                        let busy = !mixer.clips.lock().is_empty() || !mixer.tones.lock().is_empty();
                        match (busy, idle_since) {
                            (true, _) => idle_since = None,
                            (false, None) => idle_since = Some(Instant::now()),
                            (false, Some(t)) if t.elapsed() > Duration::from_secs(5) => break,
                            _ => {}
                        }
                    }
                }
                let _ = stop_tx.send(());
                if let Ok(t) = thread {
                    let _ = t.join();
                }
                alive.store(false, Ordering::SeqCst);
                // Something arrived just as we closed: go again.
                if ok && !mixer.clips.lock().is_empty() && !alive.swap(true, Ordering::SeqCst) {
                    continue;
                }
                return;
            }
        });
        if spawned.is_err() {
            self.sfx_alive.store(false, Ordering::SeqCst);
        }
    }
}

fn play_detached(tone: &Tone<'_>, device: &str) -> Result<(), String> {
    let host = cpal::default_host();
    let dev = find_device(&host, device, false).ok_or("no output device")?;
    let cfg = dev.default_output_config().map_err(|e| e.to_string())?;
    let rate = cfg.sample_rate();
    let channels = cfg.channels() as usize;
    if cfg.sample_format() != cpal::SampleFormat::F32 {
        return Err("chime: non-f32 output".into());
    }
    let samples = synth(tone, rate);
    let secs = samples.len() as f32 / rate as f32;
    let queue = Arc::new(Mutex::new(VecDeque::from(samples)));
    let q = queue.clone();
    let stream = dev
        .build_output_stream(
            cfg.into(),
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let mut q = q.lock();
                for f in data.chunks_mut(channels) {
                    let v = q.pop_front().unwrap_or(0.0);
                    f.iter_mut().for_each(|x| *x = v);
                }
            },
            |e| log::debug!("chime stream: {e}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    std::thread::sleep(Duration::from_secs_f32(secs + 0.15));
    drop(stream);
    Ok(())
}

pub enum InputSource {
    /// A capture device by name ("" = default).
    Device(String),
    /// A WAV file, looped, paced in real time (headless tests).
    Wav(String),
    /// Nothing (listen-only).
    Silence,
}

pub enum OutputSink {
    Device(String),
    /// Discard (but measure), optionally recording to a WAV file.
    Null(Option<String>),
}

struct Running {
    stop: std_mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Running {
    fn stop(mut self) {
        let _ = self.stop.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Chunk {
    rate: u32,
    mono: Vec<f32>,
    /// When it was captured.
    at: Instant,
}

pub struct AudioEngine {
    pub gate: Arc<Gate>,
    pub mixer: Arc<Mixer>,
    /// Clips played while there's no call (previews, Space's instruments):
    /// their own short-lived output stream, closed after a few idle seconds.
    sfx: Arc<Mixer>,
    sfx_alive: Arc<AtomicBool>,
    frames: broadcast::Sender<Arc<AudioFrame>>,
    input: Mutex<Option<Running>>,
    output: Mutex<Option<Running>>,
    /// Called (from audio threads) when the local speaking state flips.
    notify: Arc<dyn Fn() + Send + Sync>,
    /// "Hear yourself": the gated mic (before mute), while recording.
    recorder: Arc<Mutex<Option<Vec<f32>>>>,
    /// Health (V6): when the mic last delivered, when the speakers last asked
    /// for audio, and whether either stream reported an error.
    last_chunk: Arc<Mutex<Option<Instant>>>,
    last_render: Arc<Mutex<Option<Instant>>>,
    input_error: Arc<AtomicBool>,
    output_error: Arc<AtomicBool>,
    /// The devices actually opened (None: a WAV, silence, the null sink, or nothing).
    opened: Mutex<(Option<String>, Option<String>)>,
    /// What the call's output plays, for the echo canceller on the mic.
    echo_ref: Arc<echo::EchoRef>,
}

/// How the mic and speakers are doing (checked every ~2 s in a call).
#[derive(Debug, Default, Clone)]
pub struct Health {
    /// A stream reported an error (the device went away, the OS took it).
    pub input_error: bool,
    pub output_error: bool,
    /// Running, but nothing for over 2 s.
    pub input_stalled: bool,
    pub output_stalled: bool,
    pub opened_input: Option<String>,
    pub opened_output: Option<String>,
}

impl AudioEngine {
    pub fn new(notify: Arc<dyn Fn() + Send + Sync>) -> Arc<Self> {
        let gate = Arc::new(Gate::new());
        let new_mixer = || {
            Arc::new(Mixer {
                streams: RwLock::new(Vec::new()),
                gate: gate.clone(),
                tones: Mutex::new(VecDeque::new()),
                clips: Mutex::new(clips::Clips::default()),
                epoch: Instant::now(),
                rendered: AtomicU64::new(0),
                out_level: AtomicF32::new(0.0),
            })
        };
        let (mixer, sfx) = (new_mixer(), new_mixer());
        let (frames, _) = broadcast::channel(64);
        Arc::new(Self {
            gate,
            mixer,
            sfx,
            sfx_alive: Arc::new(AtomicBool::new(false)),
            frames,
            input: Mutex::new(None),
            output: Mutex::new(None),
            notify,
            recorder: Arc::new(Mutex::new(None)),
            last_chunk: Arc::new(Mutex::new(None)),
            last_render: Arc::new(Mutex::new(None)),
            input_error: Arc::new(AtomicBool::new(false)),
            output_error: Arc::new(AtomicBool::new(false)),
            opened: Mutex::new((None, None)),
            echo_ref: Arc::new(echo::EchoRef::default()),
        })
    }

    /// Start or stop recording the gated mic ("Hear yourself"); stopping returns it (mono 48 kHz).
    pub fn record(&self, on: bool) -> Option<Vec<f32>> {
        let mut r = self.recorder.lock();
        if on {
            *r = Some(Vec::with_capacity(RATE as usize * 5));
            None
        } else {
            r.take()
        }
    }

    pub fn health(&self) -> Health {
        let stalled = |at: &Mutex<Option<Instant>>| at.lock().is_some_and(|t| t.elapsed() > Duration::from_secs(2));
        let opened = self.opened.lock().clone();
        Health {
            input_error: self.input_running() && self.input_error.load(Ordering::Relaxed),
            output_error: self.output_running() && self.output_error.load(Ordering::Relaxed),
            input_stalled: self.input_running() && stalled(&self.last_chunk),
            output_stalled: self.output_running() && stalled(&self.last_render),
            opened_input: opened.0,
            opened_output: opened.1,
        }
    }

    /// Debug aid: act as if a stream had failed (the recovery path, without real devices).
    pub fn simulate_failure(&self, input: bool) {
        if input { &self.input_error } else { &self.output_error }.store(true, Ordering::Relaxed);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<AudioFrame>> {
        self.frames.subscribe()
    }

    pub fn input_running(&self) -> bool {
        self.input.lock().is_some()
    }

    pub fn output_running(&self) -> bool {
        self.output.lock().is_some()
    }

    pub fn add_stream(&self, sid: &str, volume: f32) -> Arc<PeerStream> {
        let s = Arc::new(PeerStream {
            sid: RwLock::new(sid.to_string()),
            jb: Mutex::new(JitterBuffer::new()),
            volume: AtomicF32::new(volume),
            limiter: Mutex::new(dsp::Limiter::new(RATE)),
            level: AtomicF32::new(0.0),
            last_loud_ms: AtomicU64::new(0),
            epoch: self.mixer.epoch,
        });
        let mut streams = self.mixer.streams.write();
        streams.retain(|x| *x.sid.read() != sid);
        streams.push(s.clone());
        s
    }

    pub fn remove_stream(&self, sid: &str) {
        self.mixer.streams.write().retain(|x| *x.sid.read() != sid);
    }

    /// Someone resumed the call under a new session: same stream, new name.
    pub fn rename_stream(&self, from: &str, to: &str) {
        let streams = self.mixer.streams.read();
        if let Some(s) = streams.iter().find(|s| *s.sid.read() == from) {
            *s.sid.write() = to.to_string();
        }
    }

    pub fn stream(&self, sid: &str) -> Option<Arc<PeerStream>> {
        self.mixer.streams.read().iter().find(|s| *s.sid.read() == sid).cloned()
    }

    pub fn streams(&self) -> Vec<Arc<PeerStream>> {
        self.mixer.streams.read().clone()
    }

    pub fn set_volume(&self, sid: &str, volume: f32) {
        if let Some(s) = self.stream(sid) {
            s.volume.set(volume);
        }
    }

    /// (Re)start capture. Returns an error string if the mic couldn't open
    /// (we then keep running "listen only": no frames, gate closed).
    pub fn start_input(&self, source: InputSource) -> Result<(), String> {
        self.stop_input();
        let (chunk_tx, chunk_rx) = std_mpsc::sync_channel::<Chunk>(64);
        let (stop_tx, stop_rx) = std_mpsc::channel::<()>();
        let (ready_tx, ready_rx) = std_mpsc::channel::<Result<Option<String>, String>>();
        self.input_error.store(false, Ordering::Relaxed);
        *self.last_chunk.lock() = None;
        let thread = match source {
            InputSource::Device(name) => {
                let failed = self.input_error.clone();
                std::thread::Builder::new()
                    .name("yap-audio-in".into())
                    .spawn(move || run_device_input(&name, chunk_tx, stop_rx, ready_tx, failed))
            }
            InputSource::Wav(path) => std::thread::Builder::new()
                .name("yap-wav-in".into())
                .spawn(move || run_wav_input(&path, chunk_tx, stop_rx, ready_tx)),
            InputSource::Silence => std::thread::Builder::new().name("yap-silence-in".into()).spawn(move || {
                let _ = ready_tx.send(Ok(None));
                let mut next = Instant::now();
                loop {
                    next += Duration::from_millis(10);
                    if stop_rx.recv_timeout(next.saturating_duration_since(Instant::now())).is_ok() {
                        return;
                    }
                    if chunk_tx.try_send(Chunk { rate: RATE, mono: vec![0.0; BLOCK], at: Instant::now() }).is_err() {
                        // processing thread gone
                    }
                }
            }),
        }
        .map_err(|e| e.to_string())?;
        let result = ready_rx.recv_timeout(Duration::from_secs(10)).unwrap_or_else(|_| Err("audio input timed out".into()));
        let running = Running { stop: stop_tx, thread: Some(thread) };
        let opened = match result {
            Ok(name) => name,
            Err(e) => {
                running.stop();
                self.gate.mic_ok.store(false, Ordering::Relaxed);
                self.opened.lock().0 = None;
                return Err(e);
            }
        };
        self.opened.lock().0 = opened;
        // Processing thread: ends when the source thread drops its sender.
        let capture = Capture {
            gate: self.gate.clone(),
            frames: self.frames.clone(),
            notify: self.notify.clone(),
            recorder: self.recorder.clone(),
            last_chunk: self.last_chunk.clone(),
            echo: self.echo_ref.clone(),
        };
        *self.last_chunk.lock() = Some(Instant::now());
        let _ = std::thread::Builder::new().name("yap-capture".into()).spawn(move || process_capture(chunk_rx, capture));
        self.gate.mic_ok.store(true, Ordering::Relaxed);
        *self.input.lock() = Some(running);
        Ok(())
    }

    pub fn stop_input(&self) {
        if let Some(r) = self.input.lock().take() {
            r.stop();
        }
        self.gate.mic_ok.store(false, Ordering::Relaxed);
        self.gate.level_db.set(-100.0);
        if self.gate.speaking.swap(false, Ordering::Relaxed) {
            (self.notify)();
        }
        self.gate.transmitting.store(false, Ordering::Relaxed);
        self.gate.silent.store(false, Ordering::Relaxed);
        *self.last_chunk.lock() = None;
        self.opened.lock().0 = None;
    }

    pub fn start_output(&self, sink: OutputSink) -> Result<(), String> {
        self.stop_output();
        let (stop_tx, stop_rx) = std_mpsc::channel::<()>();
        let (ready_tx, ready_rx) = std_mpsc::channel::<Result<Option<String>, String>>();
        let mixer = self.mixer.clone();
        self.output_error.store(false, Ordering::Relaxed);
        let alive = Some(self.last_render.clone());
        // A new stream: the echo canceller lines up with it from scratch.
        self.echo_ref.start();
        let echo = Some(self.echo_ref.clone());
        let thread = match sink {
            OutputSink::Device(name) => {
                let failed = Some(self.output_error.clone());
                std::thread::Builder::new()
                    .name("yap-audio-out".into())
                    .spawn(move || run_device_output(&name, mixer, echo, stop_rx, ready_tx, failed, alive))
            }
            OutputSink::Null(record) => std::thread::Builder::new()
                .name("yap-null-out".into())
                .spawn(move || run_null_output(record, mixer, echo, stop_rx, ready_tx, alive)),
        }
        .map_err(|e| e.to_string())?;
        let result = ready_rx.recv_timeout(Duration::from_secs(10)).unwrap_or_else(|_| Err("audio output timed out".into()));
        let running = Running { stop: stop_tx, thread: Some(thread) };
        match result {
            Ok(name) => self.opened.lock().1 = name,
            Err(e) => {
                running.stop();
                self.echo_ref.stop();
                self.opened.lock().1 = None;
                return Err(e);
            }
        }
        *self.last_render.lock() = Some(Instant::now());
        *self.output.lock() = Some(running);
        Ok(())
    }

    pub fn stop_output(&self) {
        if let Some(r) = self.output.lock().take() {
            r.stop();
        }
        self.echo_ref.stop();
        *self.last_render.lock() = None;
        self.opened.lock().1 = None;
    }

    pub fn stop(&self) {
        self.stop_input();
        self.stop_output();
    }
}

pub fn list_devices() -> (Vec<String>, Vec<String>) {
    let host = cpal::default_host();
    let names = |it: Result<_, _>| -> Vec<String> {
        match it {
            Ok(devs) => {
                let devs: Vec<cpal::Device> = devs;
                devs.into_iter().map(|d| d.to_string()).filter(|n| !n.is_empty()).collect()
            }
            Err(_) => Vec::new(),
        }
    };
    let inputs = names(host.input_devices().map(|d| d.collect()));
    let outputs = names(host.output_devices().map(|d| d.collect()));
    (dedup(inputs), dedup(outputs))
}

/// The system's default mic and speakers (names), right now.
pub fn default_devices() -> (Option<String>, Option<String>) {
    let host = cpal::default_host();
    (host.default_input_device().map(|d| d.to_string()), host.default_output_device().map(|d| d.to_string()))
}

fn dedup(mut v: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    v.retain(|x| seen.insert(x.clone()));
    v
}

fn find_device(host: &cpal::Host, name: &str, input: bool) -> Option<cpal::Device> {
    if !name.is_empty() {
        let found = if input { host.input_devices().ok() } else { host.output_devices().ok() }
            .and_then(|mut it| it.find(|d| d.to_string() == name));
        if found.is_some() {
            return found;
        }
        log::warn!("audio device {name:?} not found; using the default");
    }
    if input { host.default_input_device() } else { host.default_output_device() }
}

fn run_device_input(name: &str, tx: std_mpsc::SyncSender<Chunk>, stop: std_mpsc::Receiver<()>, ready: std_mpsc::Sender<Result<Option<String>, String>>, failed: Arc<AtomicBool>) {
    let host = cpal::default_host();
    let Some(device) = find_device(&host, name, true) else {
        let _ = ready.send(Err("No microphone found.".into()));
        return;
    };
    let supported = match pick_config(&device, true) {
        Ok(c) => c,
        Err(e) => {
            let _ = ready.send(Err(format!("Microphone unavailable: {e}")));
            return;
        }
    };
    let rate = supported.sample_rate();
    let channels = supported.channels() as usize;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    log::info!("mic: {device} @ {rate} Hz x{channels} {format:?}");
    // Any stream error (the device went away, the OS took it): the core restarts the mic.
    let err_fn = move |e| {
        log::warn!("input stream error: {e}");
        failed.store(true, Ordering::Relaxed);
    };
    macro_rules! build {
        ($t:ty) => {{
            let tx = tx.clone();
            device.build_input_stream(
                config.clone(),
                move |data: &[$t], _: &cpal::InputCallbackInfo| {
                    let mono: Vec<f32> = data
                        .chunks(channels)
                        .map(|f| f.iter().map(|s| <f32 as cpal::FromSample<$t>>::from_sample_(*s)).sum::<f32>() / channels as f32)
                        .collect();
                    let _ = tx.try_send(Chunk { rate, mono, at: Instant::now() });
                },
                err_fn.clone(),
                None,
            )
        }};
    }
    let stream = match format {
        cpal::SampleFormat::F32 => build!(f32),
        cpal::SampleFormat::I16 => build!(i16),
        cpal::SampleFormat::U16 => build!(u16),
        cpal::SampleFormat::I32 => build!(i32),
        cpal::SampleFormat::U8 => build!(u8),
        cpal::SampleFormat::F64 => build!(f64),
        other => {
            let _ = ready.send(Err(format!("Unsupported mic sample format {other:?}")));
            return;
        }
    };
    let stream = match stream.and_then(|s| s.play().map(|_| s)) {
        Ok(s) => s,
        Err(e) => {
            let _ = ready.send(Err(format!("Microphone unavailable: {e}")));
            return;
        }
    };
    let _ = ready.send(Ok(Some(device.to_string())));
    let _ = stop.recv();
    drop(stream);
}

/// Prefer 48 kHz (no resampling) when the device offers it; else its default.
fn pick_config(device: &cpal::Device, input: bool) -> Result<cpal::SupportedStreamConfig, String> {
    let default = if input { device.default_input_config() } else { device.default_output_config() }.map_err(|e| e.to_string())?;
    if default.sample_rate() == RATE {
        return Ok(default);
    }
    let ranges: Vec<cpal::SupportedStreamConfigRange> = if input {
        device.supported_input_configs().map(|r| r.collect()).unwrap_or_default()
    } else {
        device.supported_output_configs().map(|r| r.collect()).unwrap_or_default()
    };
    let want_ch = default.channels();
    let fits = |r: &&cpal::SupportedStreamConfigRange| r.min_sample_rate() <= RATE && r.max_sample_rate() >= RATE;
    if let Some(r) = ranges
        .iter()
        .filter(fits)
        .find(|r| r.channels() == want_ch && r.sample_format() == default.sample_format())
        .or_else(|| ranges.iter().filter(fits).find(|r| r.sample_format() == cpal::SampleFormat::F32))
        .or_else(|| ranges.iter().find(fits))
    {
        return Ok(r.with_sample_rate(RATE));
    }
    Ok(default)
}

/// Test aid (headless): `YAP_DEBUG_ECHO_LOOP=<ms>[,<dB>]` (default −6 dB)
/// plays what the null output renders back into the WAV mic, that much later
/// and quieter: speakers into a mic, to test echo cancellation end to end
/// without either. Its two 10 ms pacers run on their own, so the delay jitters
/// by up to 10 ms like a real device pair.
struct EchoLoop {
    air: Mutex<VecDeque<f32>>,
    gain: f32,
}

impl EchoLoop {
    fn play(&self, stereo: &[f32]) {
        let mut air = self.air.lock();
        air.extend(stereo.as_chunks::<2>().0.iter().map(|[l, r]| (l + r) * 0.5 * self.gain));
        let over = air.len().saturating_sub(RATE as usize);
        air.drain(..over);
    }

    fn hear(&self, mono: &mut [f32]) {
        let mut air = self.air.lock();
        for s in mono.iter_mut() {
            *s += air.pop_front().unwrap_or(0.0);
        }
    }
}

fn echo_loop() -> Option<&'static EchoLoop> {
    static LOOP: std::sync::OnceLock<Option<EchoLoop>> = std::sync::OnceLock::new();
    LOOP.get_or_init(|| {
        let spec = std::env::var("YAP_DEBUG_ECHO_LOOP").ok()?;
        let mut parts = spec.split(',');
        let ms: f32 = parts.next()?.trim().parse().ok()?;
        let db: f32 = parts.next().and_then(|d| d.trim().parse().ok()).unwrap_or(-6.0);
        let delay = (ms.max(0.0) / 1000.0 * RATE as f32) as usize;
        log::info!("echo loop: what's heard comes back into the mic {ms} ms later at {db} dB");
        Some(EchoLoop { air: Mutex::new(std::iter::repeat_n(0.0, delay).collect()), gain: 10f32.powf(db / 20.0) })
    })
    .as_ref()
}

fn run_wav_input(path: &str, tx: std_mpsc::SyncSender<Chunk>, stop: std_mpsc::Receiver<()>, ready: std_mpsc::Sender<Result<Option<String>, String>>) {
    let wav = match wav::read_mono(path) {
        Ok(w) if !w.samples.is_empty() => w,
        Ok(_) => {
            let _ = ready.send(Err(format!("{path}: empty")));
            return;
        }
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let _ = ready.send(Ok(None));
    let per_chunk = (wav.rate / 100) as usize; // 10 ms
    let mut pos = 0usize;
    let mut next = Instant::now();
    loop {
        next += Duration::from_millis(10);
        if stop.recv_timeout(next.saturating_duration_since(Instant::now())).is_ok() {
            return;
        }
        let mut mono = Vec::with_capacity(per_chunk);
        for _ in 0..per_chunk {
            mono.push(wav.samples[pos]);
            pos = (pos + 1) % wav.samples.len();
        }
        if let Some(room) = echo_loop().filter(|_| wav.rate == RATE) {
            room.hear(&mut mono);
        }
        if tx.try_send(Chunk { rate: wav.rate, mono, at: Instant::now() }).is_err() {
            log::debug!("capture processing is behind; dropped 10 ms");
        }
    }
}

fn process_capture(rx: std_mpsc::Receiver<Chunk>, shared: Capture) {
    let Capture { gate, frames, notify, recorder, last_chunk, echo } = shared;
    let mut canceller = echo::Canceller::new(echo);
    let mut blocks = 0u32;
    let mut resampler: Option<(u32, Resampler)> = None;
    let mut acc: Vec<f32> = Vec::with_capacity(BLOCK * 4);
    let mut resampled = Vec::with_capacity(BLOCK * 4);
    let mut highpass = dsp::HighPass::new(RATE, 80.0);
    let mut denoise = nnnoiseless::DenoiseState::new();
    let mut agc = dsp::Agc::default();
    let mut tx = gate::Gate::new(RATE, gate.tx_mode(), gate.vad_threshold.get());
    let mut work = [0f32; BLOCK];
    let mut ns_in = [0f32; BLOCK];
    let mut gated = [0f32; BLOCK];
    let mut frame = [0i16; FRAME];
    let mut frame_fill = 0usize;
    let mut frame_sq = 0f32;
    let mut frame_voice = false;
    let mut silent_samples = 0usize;
    while let Ok(chunk) = rx.recv() {
        *last_chunk.lock() = Some(Instant::now());
        // Digital silence straight from the device (before anything here touches it).
        if chunk.mono.iter().all(|x| x.abs() < 1.0 / (1u32 << 23) as f32) {
            silent_samples += chunk.mono.len() * RATE as usize / chunk.rate.max(1) as usize;
        } else {
            silent_samples = 0;
        }
        gate.silent.store(silent_samples >= RATE as usize * 5, Ordering::Relaxed);
        let input: &[f32] = if chunk.rate == RATE {
            &chunk.mono
        } else {
            if resampler.as_ref().is_none_or(|(r, _)| *r != chunk.rate) {
                resampler = Some((chunk.rate, Resampler::new(chunk.rate, RATE, 1)));
            }
            resampled.clear();
            resampler.as_mut().unwrap().1.process(&chunk.mono, &mut resampled);
            &resampled
        };
        acc.extend_from_slice(input);
        // Falling behind (a slow machine, a stall): skip noise removal until caught up.
        let behind = chunk.at.elapsed() >= BEHIND;
        gate.behind.store(behind, Ordering::Relaxed);
        let mut offset = 0;
        while acc.len() - offset >= BLOCK {
            work.copy_from_slice(&acc[offset..offset + BLOCK]);
            offset += BLOCK;
            highpass.process(&mut work);
            // Echo cancellation: every block, on or off, so it stays lined up with the speakers.
            canceller.process(gate.echo_cancellation.load(Ordering::Relaxed), &mut work);
            blocks += 1;
            if blocks.is_multiple_of(10) {
                *gate.echo.lock() = canceller.stats();
            }
            let mut voice = None;
            if gate.noise_suppression.load(Ordering::Relaxed) && !behind {
                for (d, s) in ns_in.iter_mut().zip(&work) {
                    *d = s * 32768.0;
                }
                let mut ns_out = [0f32; BLOCK];
                voice = Some(denoise.process_frame(&mut ns_out, &ns_in));
                for (d, s) in work.iter_mut().zip(&ns_out) {
                    *d = s / 32768.0;
                }
            }
            if gate.auto_gain.load(Ordering::Relaxed) {
                agc.process(&mut work, voice);
                gate.agc_db.set(agc.gain_db());
            }
            // The gate: settings from the UI / PTT poller, then 10 ms through it.
            tx.set_mode(gate.tx_mode());
            tx.set_threshold(gate.vad_threshold.get());
            tx.set_test(gate.test.load(Ordering::Relaxed));
            tx.set_ptt(gate.ptt_down.load(Ordering::Relaxed));
            if let Some(r) = tx.process(&work, &mut gated) {
                // The meter: 20 ms of level, before the gate (it keeps working while closed).
                gate.level_db.set(r.db);
                gate.open_at.set(r.at);
            }
            let db = tx.level;
            if let Some(rec) = recorder.lock().as_mut() {
                rec.extend_from_slice(&gated);
            }
            // Mute / deafen: a hard switch outside the gate.
            let on = gate.mic_ok.load(Ordering::Relaxed) && !gate.muted.load(Ordering::Relaxed) && !gate.deafened.load(Ordering::Relaxed) && tx.is_open();
            gate.transmitting.store(on, Ordering::Relaxed);
            let speaking = on && (tx.mode() == gate::Mode::Vad || db > gate.vad_threshold.get());
            if gate.speaking.swap(speaking, Ordering::Relaxed) != speaking {
                notify();
            }
            let live = gate.mic_ok.load(Ordering::Relaxed) && !gate.muted.load(Ordering::Relaxed) && !gate.deafened.load(Ordering::Relaxed);
            for &s in &gated {
                let v = if live { s } else { 0.0 };
                frame[frame_fill] = (v.clamp(-1.0, 1.0) * 32767.0) as i16;
                frame_sq += v * v;
                frame_fill += 1;
            }
            frame_voice |= speaking;
            if frame_fill == FRAME {
                let frms = (frame_sq / FRAME as f32).sqrt();
                let dbov = if frms > 0.0 { (-20.0 * frms.log10()).clamp(0.0, 127.0) } else { 127.0 };
                let _ = frames.send(Arc::new(AudioFrame { pcm: frame, level_dbov: dbov as u8, voice: frame_voice, at: Instant::now() }));
                frame_fill = 0;
                frame_sq = 0.0;
                frame_voice = false;
            }
        }
        acc.drain(..offset);
    }
    gate.transmitting.store(false, Ordering::Relaxed);
    *gate.echo.lock() = echo::EchoStats::default();
}

/// What the capture thread shares with the rest of the engine.
struct Capture {
    gate: Arc<Gate>,
    frames: broadcast::Sender<Arc<AudioFrame>>,
    notify: Arc<dyn Fn() + Send + Sync>,
    /// "Hear yourself": the gated mic (before mute) while it's recording.
    recorder: Arc<Mutex<Option<Vec<f32>>>>,
    /// When the last chunk came from the device.
    last_chunk: Arc<Mutex<Option<Instant>>>,
    /// What the speakers play (the echo canceller's far end).
    echo: Arc<echo::EchoRef>,
}

struct OutputState {
    mixer: Arc<Mixer>,
    /// The call's output: what it plays goes to the echo canceller too.
    echo: Option<Arc<echo::EchoRef>>,
    resampler: Option<Resampler>,
    fifo: VecDeque<f32>,
    block: Vec<f32>,
    scratch: Vec<f32>,
    out: Vec<f32>,
}

impl OutputState {
    fn new(mixer: Arc<Mixer>, echo: Option<Arc<echo::EchoRef>>, device_rate: u32) -> Self {
        Self {
            mixer,
            echo,
            resampler: (device_rate != RATE).then(|| Resampler::new(RATE, device_rate, 2)),
            fifo: VecDeque::with_capacity(8192),
            block: vec![0.0; BLOCK * 2],
            scratch: Vec::with_capacity(BLOCK * 2),
            out: Vec::with_capacity(BLOCK * 4),
        }
    }

    /// Stereo frames at the device rate.
    fn fill(&mut self, frames: usize) {
        while self.fifo.len() < frames * 2 {
            self.mixer.render(&mut self.block, &mut self.scratch);
            if let Some(echo) = &self.echo {
                echo.push(&self.block);
            }
            match &mut self.resampler {
                Some(r) => {
                    self.out.clear();
                    r.process(&self.block, &mut self.out);
                    self.fifo.extend(self.out.iter());
                }
                None => self.fifo.extend(self.block.iter()),
            }
        }
    }
}

fn run_device_output(
    name: &str,
    mixer: Arc<Mixer>,
    echo: Option<Arc<echo::EchoRef>>,
    stop: std_mpsc::Receiver<()>,
    ready: std_mpsc::Sender<Result<Option<String>, String>>,
    failed: Option<Arc<AtomicBool>>,
    alive: Option<Arc<Mutex<Option<Instant>>>>,
) {
    let host = cpal::default_host();
    let Some(device) = find_device(&host, name, false) else {
        let _ = ready.send(Err("No speakers or headphones found.".into()));
        return;
    };
    let supported = match pick_config(&device, false) {
        Ok(c) => c,
        Err(e) => {
            let _ = ready.send(Err(format!("Audio output unavailable: {e}")));
            return;
        }
    };
    let rate = supported.sample_rate();
    let channels = supported.channels() as usize;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    log::info!("speakers: {device} @ {rate} Hz x{channels} {format:?}");
    let err_fn = move |e| {
        log::warn!("output stream error: {e}");
        if let Some(f) = &failed {
            f.store(true, Ordering::Relaxed);
        }
    };
    macro_rules! build {
        ($t:ty) => {{
            let mut st = OutputState::new(mixer.clone(), echo.clone(), rate);
            let alive = alive.clone();
            device.build_output_stream(
                config.clone(),
                move |data: &mut [$t], _: &cpal::OutputCallbackInfo| {
                    let frames = data.len() / channels;
                    st.fill(frames);
                    if let Some(a) = &alive {
                        *a.lock() = Some(Instant::now());
                    }
                    for f in data.chunks_mut(channels) {
                        let l = st.fifo.pop_front().unwrap_or(0.0);
                        let r = st.fifo.pop_front().unwrap_or(0.0);
                        if channels == 1 {
                            f[0] = <$t as cpal::FromSample<f32>>::from_sample_((l + r) * 0.5);
                        } else {
                            f[0] = <$t as cpal::FromSample<f32>>::from_sample_(l);
                            f[1] = <$t as cpal::FromSample<f32>>::from_sample_(r);
                            for x in f.iter_mut().skip(2) {
                                *x = <$t as cpal::FromSample<f32>>::from_sample_(0.0);
                            }
                        }
                    }
                },
                err_fn.clone(),
                None,
            )
        }};
    }
    let stream = match format {
        cpal::SampleFormat::F32 => build!(f32),
        cpal::SampleFormat::I16 => build!(i16),
        cpal::SampleFormat::U16 => build!(u16),
        cpal::SampleFormat::I32 => build!(i32),
        cpal::SampleFormat::U8 => build!(u8),
        cpal::SampleFormat::F64 => build!(f64),
        other => {
            let _ = ready.send(Err(format!("Unsupported output sample format {other:?}")));
            return;
        }
    };
    let stream = match stream.and_then(|s| s.play().map(|_| s)) {
        Ok(s) => s,
        Err(e) => {
            let _ = ready.send(Err(format!("Audio output unavailable: {e}")));
            return;
        }
    };
    let _ = ready.send(Ok(Some(device.to_string())));
    let _ = stop.recv();
    drop(stream);
}

fn run_null_output(
    record: Option<String>,
    mixer: Arc<Mixer>,
    echo: Option<Arc<echo::EchoRef>>,
    stop: std_mpsc::Receiver<()>,
    ready: std_mpsc::Sender<Result<Option<String>, String>>,
    alive: Option<Arc<Mutex<Option<Instant>>>>,
) {
    let mut writer = match record.as_deref().map(|p| wav::WavWriter::create(p, RATE, 2)) {
        Some(Ok(w)) => Some(w),
        Some(Err(e)) => {
            let _ = ready.send(Err(e));
            return;
        }
        None => None,
    };
    let _ = ready.send(Ok(None));
    let mut block = vec![0f32; BLOCK * 2];
    let mut scratch = Vec::new();
    let mut next = Instant::now();
    loop {
        next += Duration::from_millis(10);
        if stop.recv_timeout(next.saturating_duration_since(Instant::now())).is_ok() {
            break;
        }
        mixer.render(&mut block, &mut scratch);
        if let Some(e) = &echo {
            e.push(&block);
        }
        if let Some(room) = echo_loop() {
            room.play(&block);
        }
        if let Some(a) = &alive {
            *a.lock() = Some(Instant::now());
        }
        if let Some(w) = writer.as_mut() {
            w.write(&block);
        }
    }
    if let Some(w) = writer {
        w.finish();
    }
}
