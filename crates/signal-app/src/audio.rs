//! Voice-note / audio attachment playback on a dedicated thread.
//!
//! The output device is opened lazily on first play, so machines without an
//! audio device (CI, servers) only see an error when the user presses play.

use std::io::Cursor;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::Source as _;

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

/// Sample rate of recorded voice notes (speech quality, ~1.9 MB/min as WAV).
pub const VOICE_NOTE_RATE: u32 = 16_000;

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

    /// Stops and returns the recording as a 16 kHz mono WAV.
    pub fn finish(mut self) -> Vec<u8> {
        self.halt();
        let samples = std::mem::take(&mut *self.samples.lock().unwrap());
        encode_wav(
            &resample(&samples, self.rate, VOICE_NOTE_RATE),
            VOICE_NOTE_RATE,
        )
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
    fn encoded_wav_decodes_with_expected_duration() {
        let one_second = vec![0.25f32; VOICE_NOTE_RATE as usize];
        let wav = encode_wav(&one_second, VOICE_NOTE_RATE);
        assert_eq!(wav.len(), 44 + 2 * VOICE_NOTE_RATE as usize);
        let d = clip_duration(wav).unwrap();
        assert!((d.as_millis() as i64 - 1000).abs() <= 5, "{d:?}");
    }
}
