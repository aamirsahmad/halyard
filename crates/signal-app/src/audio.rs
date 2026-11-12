//! Voice-note / audio attachment playback on a dedicated thread.
//!
//! The output device is opened lazily on first play, so machines without an
//! audio device (CI, servers) only see an error when the user presses play.

use std::borrow::Cow;
use std::io::Cursor;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::Source as _;

use crate::settings::Ringtone;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaybackStatus {
    /// Key of the attachment being played (message id, attachment index).
    pub current: Option<(i64, usize)>,
    pub paused: bool,
    pub position: Duration,
    pub duration: Option<Duration>,
    pub error: Option<String>,
}

enum Cmd {
    Play { key: (i64, usize), bytes: Vec<u8> },
    TogglePause,
    Stop,
}

pub struct AudioPlayer {
    tx: mpsc::Sender<Cmd>,
    status: Arc<Mutex<PlaybackStatus>>,
}

/// Duration of an encoded audio clip, if the decoder can tell.
pub fn clip_duration(bytes: Vec<u8>) -> Option<Duration> {
    rodio::Decoder::new(Cursor::new(bytes))
        .ok()?
        .total_duration()
}

/// "m:ss" formatting used next to the play button.
pub fn format_clock(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

impl Default for AudioPlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioPlayer {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<Cmd>();
        let status: Arc<Mutex<PlaybackStatus>> = Arc::default();
        let shared = status.clone();
        let spawned = std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || {
                let mut sink: Option<rodio::MixerDeviceSink> = None;
                let mut player: Option<rodio::Player> = None;
                loop {
                    match rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(Cmd::Play { key, bytes }) => {
                            if let Some(p) = player.take() {
                                p.stop();
                            }
                            let result =
                                (|| -> Result<(rodio::Player, Option<Duration>), String> {
                                    if sink.is_none() {
                                        sink = Some(
                                            rodio::DeviceSinkBuilder::open_default_sink()
                                                .map_err(|e| format!("No audio output: {e}"))?,
                                        );
                                    }
                                    let source = rodio::Decoder::new(Cursor::new(bytes))
                                        .map_err(|e| format!("Unsupported audio format: {e}"))?;
                                    let duration = source.total_duration();
                                    let p = rodio::Player::connect_new(
                                        sink.as_ref().expect("opened").mixer(),
                                    );
                                    p.append(source);
                                    Ok((p, duration))
                                })();
                            let mut st = shared.lock().unwrap();
                            match result {
                                Ok((p, duration)) => {
                                    player = Some(p);
                                    *st = PlaybackStatus {
                                        current: Some(key),
                                        duration,
                                        ..Default::default()
                                    };
                                }
                                Err(e) => {
                                    tracing::warn!(%e, "audio playback failed");
                                    *st = PlaybackStatus {
                                        error: Some(e),
                                        ..Default::default()
                                    };
                                }
                            }
                        }
                        Ok(Cmd::TogglePause) => {
                            if let Some(p) = &player {
                                if p.is_paused() {
                                    p.play()
                                } else {
                                    p.pause()
                                }
                                shared.lock().unwrap().paused = p.is_paused();
                            }
                        }
                        Ok(Cmd::Stop) => {
                            if let Some(p) = player.take() {
                                p.stop();
                            }
                            *shared.lock().unwrap() = PlaybackStatus::default();
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                    if let Some(p) = &player {
                        let mut st = shared.lock().unwrap();
                        if p.empty() {
                            player = None;
                            *st = PlaybackStatus::default();
                        } else {
                            st.position = p.get_pos();
                        }
                    }
                }
            });
        if let Err(e) = spawned {
            tracing::error!(%e, "failed to start audio thread");
        }
        Self { tx, status }
    }

    pub fn status(&self) -> PlaybackStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn is_active(&self) -> bool {
        self.status.lock().unwrap().current.is_some()
    }

    /// Plays `bytes` for attachment `key`, or toggles pause if it is the
    /// current clip.
    pub fn toggle(&self, key: (i64, usize), bytes: impl FnOnce() -> Option<Vec<u8>>) {
        if self.status().current == Some(key) {
            let _ = self.tx.send(Cmd::TogglePause);
            return;
        }
        match bytes() {
            Some(bytes) => {
                let _ = self.tx.send(Cmd::Play { key, bytes });
            }
            None => self.status.lock().unwrap().error = Some("Couldn't read the audio file".into()),
        }
    }

    pub fn stop(&self) {
        let _ = self.tx.send(Cmd::Stop);
    }

    /// Takes the last error (shown once as a notification).
    pub fn take_error(&self) -> Option<String> {
        self.status.lock().unwrap().error.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ringtones_are_quiet_loops() {
        for tone in Ringtone::ALL {
            let Some(samples) = ringtone_samples(tone) else {
                assert!(matches!(tone, Ringtone::Signal | Ringtone::Silent));
                continue;
            };
            let secs = samples.len() as f32 / SYNTH_RATE as f32;
            assert!((2.0..=4.0).contains(&secs), "{tone:?}: {secs}s");
            let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
            assert!(peak > 0.1 && peak < 0.9, "{tone:?} peak {peak}");
            // the round ends in silence, so looping doesn't click
            assert!(samples[samples.len() - 100..].iter().all(|s| *s == 0.));
            let wav = ringtone_bytes(tone).unwrap();
            // RINGTONE_DIR=<dir> saves the tones to listen to them
            if let Ok(dir) = std::env::var("RINGTONE_DIR") {
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(format!("{dir}/{}.wav", tone.label()), &wav).unwrap();
            }
            let d = rodio::Decoder::new(Cursor::new(wav)).expect("decodes");
            assert!(d.take(100).count() == 100);
        }
        assert!(ringtone_bytes(Ringtone::Silent).is_none());
    }

    #[test]
    fn call_sounds_decode() {
        for bytes in [RINGTONE, END_CALL] {
            let d = rodio::Decoder::new(Cursor::new(bytes)).expect("decodes");
            assert!(d.total_duration().is_none_or(|t| t > Duration::ZERO));
            assert!(d.take(100).count() == 100, "has samples");
        }
    }

    /// A 0.5 s, 8 kHz mono 16-bit PCM WAV of silence.
    fn wav() -> Vec<u8> {
        let samples = 4000u32;
        let data_len = samples * 2;
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data_len).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes()); // PCM
        v.extend_from_slice(&1u16.to_le_bytes()); // mono
        v.extend_from_slice(&8000u32.to_le_bytes());
        v.extend_from_slice(&16000u32.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&data_len.to_le_bytes());
        v.resize(v.len() + data_len as usize, 0);
        v
    }

    #[test]
    fn decodes_duration() {
        let d = clip_duration(wav()).unwrap();
        assert!((d.as_millis() as i64 - 500).abs() <= 5, "{d:?}");
        assert!(clip_duration(b"not audio".to_vec()).is_none());
    }

    #[test]
    fn clock_format() {
        assert_eq!(format_clock(Duration::from_secs(5)), "0:05");
        assert_eq!(format_clock(Duration::from_secs(125)), "2:05");
    }

    #[test]
    fn unreadable_file_reports_error_without_device() {
        let p = AudioPlayer::new();
        p.toggle((1, 0), || None);
        assert_eq!(
            p.take_error().as_deref(),
            Some("Couldn't read the audio file")
        );
        assert!(!p.is_active());
    }
}

/// Sample rate of the WAV fallback (speech quality, ~1.9 MB/min).
pub const VOICE_NOTE_RATE: u32 = 16_000;

/// Encodes mono samples as MP3 the way Signal Desktop records voice notes
/// (LAME, ~90 kbps average, mono). The VBR header is rewritten at the end so
/// players know the real duration.
pub fn encode_mp3(samples: &[f32], rate: u32) -> Result<Vec<u8>, String> {
    use mp3lame_encoder::{
        Bitrate, Builder, FlushNoGap, Mode, MonoPcm, Quality, VbrMode, max_required_buffer_size,
    };
    let err = |e: &dyn std::fmt::Display| format!("MP3 encoding failed: {e}");
    let mut b = Builder::new().ok_or("MP3 encoder unavailable")?;
    b.set_num_channels(1).map_err(|e| err(&e))?;
    b.set_sample_rate(rate).map_err(|e| err(&e))?;
    b.set_mode(Mode::Mono).map_err(|e| err(&e))?;
    b.set_vbr_mode(VbrMode::Abr).map_err(|e| err(&e))?;
    b.set_brate(Bitrate::Kbps96).map_err(|e| err(&e))?;
    b.set_quality(Quality::Ok).map_err(|e| err(&e))?;
    b.set_to_write_vbr_tag(true).map_err(|e| err(&e))?;
    let mut encoder = b.build().map_err(|e| err(&e))?;
    let mut out = Vec::with_capacity(max_required_buffer_size(samples.len()));
    encoder
        .encode_to_vec(MonoPcm(samples), &mut out)
        .map_err(|e| err(&e))?;
    out.reserve(7200);
    encoder
        .flush_to_vec::<FlushNoGap>(&mut out)
        .map_err(|e| err(&e))?;
    let mut tag = Vec::with_capacity(encoder.lame_tag_size());
    if let Some(n) = encoder.lame_tag_encode_to_vec(&mut tag) {
        let at = encoder.id3v2_tag_size();
        if let Some(slot) = out.get_mut(at..at + n.get()) {
            slot.copy_from_slice(&tag[..n.get()]);
        }
    }
    Ok(out)
}

/// Linear resampling of mono samples from `from` Hz to `to` Hz.
pub fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() || from == 0 {
        return samples.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let n = (samples.len() as f64 / ratio).floor() as usize;
    (0..n)
        .map(|i| {
            let pos = i as f64 * ratio;
            let i0 = pos.floor() as usize;
            let i1 = (i0 + 1).min(samples.len() - 1);
            let t = (pos - i0 as f64) as f32;
            samples[i0] * (1. - t) + samples[i1] * t
        })
        .collect()
}

/// 16-bit PCM mono WAV.
pub fn encode_wav(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut v = Vec::with_capacity(44 + data_len as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&1u16.to_le_bytes()); // mono
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * 2).to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        v.extend_from_slice(&((s.clamp(-1., 1.) * i16::MAX as f32) as i16).to_le_bytes());
    }
    v
}

/// Records from the default microphone until finished or cancelled.
pub struct Recorder {
    stop: mpsc::Sender<()>,
    samples: Arc<Mutex<Vec<f32>>>,
    rate: u32,
    started: std::time::Instant,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Recorder {
    pub fn start() -> Result<Self, String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let samples: Arc<Mutex<Vec<f32>>> = Arc::default();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, String>>();
        let (stop, stop_rx) = mpsc::channel::<()>();
        let sink = samples.clone();
        let thread = std::thread::Builder::new()
            .name("recorder".into())
            .spawn(move || {
                let setup = (|| -> Result<(cpal::Stream, u32), String> {
                    let device = cpal::default_host()
                        .default_input_device()
                        .ok_or("No microphone found")?;
                    let config = device
                        .default_input_config()
                        .map_err(|e| format!("Microphone unavailable: {e}"))?;
                    let channels = config.channels().max(1) as usize;
                    let rate = config.sample_rate();
                    let err = |e: cpal::Error| tracing::warn!(%e, "recording error");
                    macro_rules! stream {
                        ($t:ty) => {{
                            let sink = sink.clone();
                            device.build_input_stream::<$t, _, _>(
                                config.config(),
                                move |data: &[$t], _: &_| {
                                    let mut out = sink.lock().unwrap();
                                    for frame in data.chunks(channels) {
                                        let sum: f32 = frame
                                            .iter()
                                            .map(|s| cpal::Sample::to_sample::<f32>(*s))
                                            .sum();
                                        out.push(sum / channels as f32);
                                    }
                                },
                                err,
                                None,
                            )
                        }};
                    }
                    let stream = match config.sample_format() {
                        cpal::SampleFormat::F32 => stream!(f32),
                        cpal::SampleFormat::I16 => stream!(i16),
                        cpal::SampleFormat::U16 => stream!(u16),
                        cpal::SampleFormat::I32 => stream!(i32),
                        other => return Err(format!("Unsupported microphone format {other:?}")),
                    }
                    .map_err(|e| format!("Couldn't open the microphone: {e}"))?;
                    stream
                        .play()
                        .map_err(|e| format!("Couldn't start recording: {e}"))?;
                    Ok((stream, rate))
                })();
                match setup {
                    Ok((stream, rate)) => {
                        let _ = ready_tx.send(Ok(rate));
                        let _ = stop_rx.recv(); // keep the stream alive until stopped
                        drop(stream);
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let rate = ready_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "Timed out opening the microphone".to_string())??;
        Ok(Self {
            stop,
            samples,
            rate,
            started: std::time::Instant::now(),
            thread: Some(thread),
        })
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    fn halt(&mut self) {
        let _ = self.stop.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    /// Stops and returns the recording as MP3 (with its file extension),
    /// falling back to a 16 kHz WAV if the encoder fails.
    pub fn finish(mut self) -> (Vec<u8>, &'static str) {
        self.halt();
        let samples = std::mem::take(&mut *self.samples.lock().unwrap());
        match encode_mp3(&samples, self.rate) {
            Ok(mp3) => (mp3, "mp3"),
            Err(e) => {
                tracing::warn!(%e, "falling back to WAV");
                let wav = encode_wav(
                    &resample(&samples, self.rate, VOICE_NOTE_RATE),
                    VOICE_NOTE_RATE,
                );
                (wav, "wav")
            }
        }
    }

    pub fn cancel(mut self) {
        self.halt();
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.halt();
    }
}

#[cfg(test)]
mod recording_tests {
    use super::*;

    #[test]
    fn resampling_halves_and_keeps_shape() {
        let s: Vec<f32> = (0..48).map(|i| i as f32).collect();
        let r = resample(&s, 48_000, 16_000);
        assert_eq!(r.len(), 16);
        assert_eq!(r[1], 3.0);
        assert_eq!(resample(&s, 16_000, 16_000), s);
        assert!(resample(&[], 48_000, 16_000).is_empty());
    }

    #[test]
    fn mp3_voice_note_is_compact_and_reports_its_duration() {
        // 3 s of a 440 Hz tone at a typical microphone rate
        let rate = 48_000;
        let tone: Vec<f32> = (0..rate * 3)
            .map(|i| (i as f32 * 440. * std::f32::consts::TAU / rate as f32).sin() * 0.3)
            .collect();
        let mp3 = encode_mp3(&tone, rate).unwrap();
        // ~96 kbps → ~36 KB for 3 s, vs 288 KB as 48 kHz WAV
        assert!(mp3.len() < 60_000, "{} bytes", mp3.len());
        assert!(mp3.len() > 10_000, "{} bytes", mp3.len());
        // the VBR (Xing/Info) header was written into the first frame
        assert!(
            mp3.windows(4)
                .take(200)
                .any(|w| w == b"Xing" || w == b"Info"),
            "missing VBR header"
        );
        let d = clip_duration(mp3).expect("decodable");
        assert!((d.as_millis() as i64 - 3000).abs() < 150, "{d:?}");
    }

    #[test]
    fn encoded_wav_decodes_with_expected_duration() {
        let one_second = vec![0.25f32; VOICE_NOTE_RATE as usize];
        let wav = encode_wav(&one_second, VOICE_NOTE_RATE);
        assert_eq!(wav.len(), 44 + 2 * VOICE_NOTE_RATE as usize);
        let d = clip_duration(wav).unwrap();
        assert!((d.as_millis() as i64 - 1000).abs() <= 5, "{d:?}");
    }
}

/// Signal Desktop's call sounds.
const RINGTONE: &[u8] = include_bytes!("../assets/sounds/ringtone_minimal.ogg");
const END_CALL: &[u8] = include_bytes!("../assets/sounds/navigation-cancel.ogg");

/// A call sound to start or stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallSound {
    /// Loop a ringtone (an incoming call rings).
    Ring(Ringtone),
    /// Play one round of a ringtone (choosing one in Preferences).
    Preview(Ringtone),
    /// Stop the ringtone.
    StopRing,
    /// Play the hang-up tone once.
    End,
}

const SYNTH_RATE: u32 = 44_100;

/// One round of a ringtone as mono samples at 44.1 kHz, ending in silence so
/// that it can loop. `None` for Signal's recorded ringtone and for Silent.
pub fn ringtone_samples(tone: Ringtone) -> Option<Vec<f32>> {
    use std::f32::consts::TAU;
    let rate = SYNTH_RATE as f32;
    let mut out: Vec<f32> = Vec::new();
    // adds a note at `start` seconds: frequencies with relative amplitudes,
    // shaped by `env(t)`
    let mut note = |start: f32, len: f32, partials: &[(f32, f32)], env: &dyn Fn(f32) -> f32| {
        let first = (start * rate) as usize;
        let n = (len * rate) as usize;
        if out.len() < first + n {
            out.resize(first + n, 0.);
        }
        for i in 0..n {
            let t = i as f32 / rate;
            // 5 ms fade in and out against clicks
            let edge = (t / 0.005).min((len - t) / 0.005).clamp(0., 1.);
            let v: f32 = partials.iter().map(|(f, a)| a * (TAU * f * t).sin()).sum();
            out[first + i] += v * env(t) * edge;
        }
    };
    let total = match tone {
        Ringtone::Signal | Ringtone::Silent => return None,
        Ringtone::Classic => {
            // the telephone double ring: 400 + 450 Hz, on 0.4 s, off 0.2 s, on 0.4 s
            let flat = |_: f32| 0.22;
            note(0.0, 0.4, &[(400., 1.), (450., 1.)], &flat);
            note(0.6, 0.4, &[(400., 1.), (450., 1.)], &flat);
            3.0
        }
        Ringtone::Chime => {
            // E6, C6, G5 bells
            let bell = |t: f32| 0.32 * (-t * 3.5).exp();
            for (i, f) in [1318.5_f32, 1046.5, 784.0].into_iter().enumerate() {
                note(
                    i as f32 * 0.4,
                    1.2,
                    &[(f, 1.), (f * 2.76, 0.18), (f * 5.4, 0.06)],
                    &bell,
                );
            }
            3.2
        }
        Ringtone::Marimba => {
            // C5 E5 G5 C6 E6 G6, twice
            let mallet = |t: f32| 0.34 * (-t * 9.).exp();
            let run = [523.25_f32, 659.25, 783.99, 1046.5, 1318.5, 1568.0];
            for round in 0..2 {
                for (i, f) in run.into_iter().enumerate() {
                    let at = round as f32 * 0.9 + i as f32 * 0.12;
                    note(at, 0.5, &[(f, 1.), (f * 4., 0.12)], &mallet);
                }
            }
            3.2
        }
        Ringtone::SciFiPiano => {
            // A minor add9, rising then settling: A3 E4 B4 C5 E5, then C5 B4 E4 A4.
            // A soft piano tone, a second voice detuned by 6 cents, and a slow
            // shimmer; an echo is added below.
            let piano = |t: f32| {
                let attack = (t / 0.012).min(1.);
                let shimmer = 1. + 0.12 * (std::f32::consts::TAU * 4.5 * t).sin();
                0.16 * attack * (-t * 2.4).exp() * shimmer
            };
            let notes = [
                (0.00, 220.00),
                (0.22, 329.63),
                (0.44, 493.88),
                (0.66, 523.25),
                (0.88, 659.26),
                (1.32, 523.25),
                (1.54, 493.88),
                (1.76, 329.63),
                (1.98, 440.00),
            ];
            for (at, f) in notes {
                let partials = [
                    (f, 1.),
                    (f * 1.0035, 0.6),
                    (f * 2., 0.28),
                    (f * 3., 0.08),
                    (f * 4.01, 0.04),
                ];
                note(at, 1.3, &partials, &piano);
            }
            4.0
        }
        Ringtone::Pulse => {
            // three soft 880 Hz beeps
            let soft = |_: f32| 0.2;
            for i in 0..3 {
                note(i as f32 * 0.25, 0.13, &[(880., 1.), (1760., 0.15)], &soft);
            }
            2.2
        }
    };
    out.resize((total * rate) as usize, 0.);
    if tone == Ringtone::SciFiPiano {
        // a soft echo (0.33 s, fading), then a fade out so the loop is clean
        let delay = (0.33 * rate) as usize;
        for i in delay..out.len() {
            out[i] += out[i - delay] * 0.38;
        }
        let fade_end = out.len() - (0.25 * rate) as usize;
        let fade_start = fade_end - (0.6 * rate) as usize;
        for (i, s) in out.iter_mut().enumerate().skip(fade_start) {
            *s *= if i >= fade_end {
                0.
            } else {
                1. - (i - fade_start) as f32 / (fade_end - fade_start) as f32
            };
        }
    }
    Some(out)
}

/// The sound file for a ringtone, if it makes a sound.
fn ringtone_bytes(tone: Ringtone) -> Option<Cow<'static, [u8]>> {
    match tone {
        Ringtone::Signal => Some(Cow::Borrowed(RINGTONE)),
        Ringtone::Silent => None,
        _ => ringtone_samples(tone).map(|s| Cow::Owned(encode_wav(&s, SYNTH_RATE))),
    }
}

/// Plays call sounds on its own thread. The output device opens on the
/// first sound.
pub struct CallTones {
    tx: mpsc::Sender<CallSound>,
}

impl Default for CallTones {
    fn default() -> Self {
        Self::new()
    }
}

impl CallTones {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<CallSound>();
        let spawned = std::thread::Builder::new()
            .name("call-tones".into())
            .spawn(move || {
                let mut sink: Option<rodio::MixerDeviceSink> = None;
                let mut ring: Option<rodio::Player> = None;
                let mut once: Option<rodio::Player> = None;
                while let Ok(sound) = rx.recv() {
                    if let Some(p) = ring.take() {
                        p.stop();
                    }
                    let bytes = match sound {
                        CallSound::StopRing => continue,
                        CallSound::Ring(t) | CallSound::Preview(t) => match ringtone_bytes(t) {
                            Some(b) => b,
                            None => continue,
                        },
                        CallSound::End => Cow::Borrowed(END_CALL),
                    };
                    let result = (|| -> Result<rodio::Player, String> {
                        if sink.is_none() {
                            sink = Some(
                                rodio::DeviceSinkBuilder::open_default_sink()
                                    .map_err(|e| format!("no audio output: {e}"))?,
                            );
                        }
                        let source = rodio::Decoder::new(Cursor::new(bytes))
                            .map_err(|e| format!("bad sound: {e}"))?;
                        let player =
                            rodio::Player::connect_new(sink.as_ref().expect("opened").mixer());
                        if matches!(sound, CallSound::Ring(_)) {
                            player.append(source.repeat_infinite());
                        } else {
                            player.append(source);
                        }
                        Ok(player)
                    })();
                    match result {
                        // a preview stops with the next sound, like the ringtone
                        Ok(p) if matches!(sound, CallSound::Ring(_) | CallSound::Preview(_)) => {
                            ring = Some(p)
                        }
                        Ok(p) => once = Some(p),
                        Err(e) => tracing::warn!(%e, "call sound failed"),
                    }
                }
                drop(once);
            });
        if let Err(e) = spawned {
            tracing::error!(%e, "failed to start call tone thread");
        }
        Self { tx }
    }

    pub fn play(&self, sound: CallSound) {
        let _ = self.tx.send(sound);
    }
}
