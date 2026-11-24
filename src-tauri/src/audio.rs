use crate::voice::{
    Decision, Endpoint, TurnEncoder, VoiceProcessor, FRAME_SAMPLES, PRE_ROLL_SAMPLES,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc::Sender, oneshot};

pub enum Control {
    Start,
    Stop,
    Abort,
    Call(bool),
    Text(String),
}
struct Request {
    control: Control,
    reply: oneshot::Sender<Result<(), String>>,
}
struct Capture {
    epoch: u32,
    generation: u32,
    samples: Vec<f32>,
    captured: Instant,
    delay_ms: u32,
}
struct Render {
    generation: u32,
    samples: Vec<f32>,
    delay_ms: u32,
}

pub struct Audio {
    pub listening: Arc<AtomicBool>,
    pub running: Arc<AtomicBool>,
    pub volume: Arc<AtomicU32>,
    pub epoch: Arc<AtomicU32>,
    pub suppressed: Arc<AtomicBool>,
    pub response_expected: Arc<AtomicBool>,
    pub call_enabled: Arc<AtomicBool>,
    pub silence_ms: Arc<AtomicU32>,
    pub playback: Arc<Mutex<VecDeque<f32>>>,
    pub incoming: mpsc::SyncSender<Vec<u8>>,
    controls: mpsc::SyncSender<Request>,
}
impl Audio {
    pub async fn command(&self, control: Control) -> Result<(), String> {
        if !self.running.load(Ordering::Relaxed) {
            return Err("音频设备不可用，请重新连接".into());
        }
        let (reply, rx) = oneshot::channel();
        self.controls
            .try_send(Request { control, reply })
            .map_err(|_| "音频操作队列不可用")?;
        tokio::time::timeout(Duration::from_secs(3), rx)
            .await
            .map_err(|_| "音频操作超时")?
            .map_err(|_| "音频线程已停止")?
    }
}
impl Drop for Audio {
    fn drop(&mut self) {
        self.call_enabled.store(false, Ordering::Relaxed);
        self.running.store(false, Ordering::Relaxed);
        self.listening.store(false, Ordering::Relaxed);
    }
}

pub fn devices() -> serde_json::Value {
    let host = cpal::default_host();
    let inputs: Vec<String> = host
        .input_devices()
        .map(|d| d.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default();
    let outputs: Vec<String> = host
        .output_devices()
        .map(|d| d.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default();
    json!({"inputs":inputs,"outputs":outputs})
}
fn select_device(input: bool, name: &str) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    if name.is_empty() {
        return (if input {
            host.default_input_device()
        } else {
            host.default_output_device()
        })
        .ok_or("没有可用的音频设备".into());
    }
    let mut all = if input {
        host.input_devices()
    } else {
        host.output_devices()
    }
    .map_err(|e| e.to_string())?;
    all.find(|d| d.name().ok().as_deref() == Some(name))
        .ok_or("所选音频设备已断开，请重新选择".into())
}

#[derive(Clone)]
struct Flags {
    listening: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    call: Arc<AtomicBool>,
    generation: Arc<AtomicU32>,
    epoch: Arc<AtomicU32>,
}
fn input_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    flags: Flags,
    tx: mpsc::SyncSender<Capture>,
    app: AppHandle,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    device
        .build_input_stream(
            config,
            move |data: &[T], info| {
                if flags.running.load(Ordering::Relaxed)
                    && (flags.call.load(Ordering::Relaxed)
                        || flags.listening.load(Ordering::Relaxed))
                {
                    let samples = data
                        .chunks_exact(channels)
                        .map(|f| {
                            f.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / channels as f32
                        })
                        .collect();
                    let stamp = info.timestamp();
                    let delay_ms = stamp
                        .callback
                        .duration_since(&stamp.capture)
                        .map(|d| d.as_millis().min(200) as u32)
                        .unwrap_or(10);
                    // A bounded callback queue; it never waits for encoding, VAD or the network.
                    let _ = tx.try_send(Capture {
                        epoch: flags.epoch.load(Ordering::Relaxed),
                        generation: flags.generation.load(Ordering::Relaxed),
                        samples,
                        captured: Instant::now(),
                        delay_ms,
                    });
                }
            },
            move |_| {
                let _ = app.emit(
                    "device-event",
                    json!({"type":"audio_error","message":"麦克风已断开，请重新连接设备"}),
                );
            },
            None,
        )
        .map_err(|e| e.to_string())
}
fn output_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    queue: Arc<Mutex<VecDeque<f32>>>,
    volume: Arc<AtomicU32>,
    flags: Flags,
    reference: mpsc::SyncSender<Render>,
    app: AppHandle,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    device
        .build_output_stream(
            config,
            move |data: &mut [T], info| {
                let capture_reference = flags.call.load(Ordering::Relaxed);
                let mut rendered = if capture_reference {
                    Vec::with_capacity(data.len() / channels)
                } else {
                    Vec::new()
                };
                if let Ok(mut samples) = queue.try_lock() {
                    let gain = volume.load(Ordering::Relaxed) as f32 / 100.0;
                    for frame in data.chunks_mut(channels) {
                        let value = samples.pop_front().unwrap_or(0.0) * gain;
                        for sample in frame {
                            *sample = T::from_sample(value);
                        }
                        if capture_reference {
                            rendered.push(value);
                        }
                    }
                } else {
                    data.fill(T::EQUILIBRIUM);
                    if capture_reference {
                        rendered.resize(data.len() / channels, 0.0);
                    }
                }
                if capture_reference {
                    let stamp = info.timestamp();
                    let delay_ms = stamp
                        .playback
                        .duration_since(&stamp.callback)
                        .map(|d| d.as_millis().min(200) as u32)
                        .unwrap_or(10);
                    let _ = reference.try_send(Render {
                        generation: flags.generation.load(Ordering::Relaxed),
                        samples: rendered,
                        delay_ms,
                    });
                }
            },
            move |_| {
                let _ = app.emit(
                    "device-event",
                    json!({"type":"audio_error","message":"扬声器已断开，请重新连接设备"}),
                );
            },
            None,
        )
        .map_err(|e| e.to_string())
}

pub struct Resampler {
    buffer: Vec<f32>,
    position: f64,
    step: f64,
}
impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        Self {
            buffer: Vec::new(),
            position: 0.0,
            step: from as f64 / to as f64,
        }
    }
    pub fn push(&mut self, data: &[f32]) -> Vec<f32> {
        self.buffer.extend_from_slice(data);
        let mut out = Vec::new();
        while self.position + 1.0 < self.buffer.len() as f64 {
            let i = self.position as usize;
            let f = (self.position - i as f64) as f32;
            out.push(self.buffer[i] * (1.0 - f) + self.buffer[i + 1] * f);
            self.position += self.step;
        }
        let used = (self.position as usize).min(self.buffer.len().saturating_sub(1));
        self.buffer.drain(..used);
        self.position -= used as f64;
        out
    }
}

struct Worker {
    app: AppHandle,
    outgoing: Sender<super::Outgoing>,
    flags: Flags,
    suppressed: Arc<AtomicBool>,
    response_expected: Arc<AtomicBool>,
    playback: Arc<Mutex<VecDeque<f32>>>,
    silence_ms: Arc<AtomicU32>,
    voice: VoiceProcessor,
    endpoint: Endpoint,
    encoder: TurnEncoder,
    pre_roll: VecDeque<f32>,
}
impl Worker {
    fn send(&self, message: super::Outgoing) -> Result<(), String> {
        self.outgoing
            .try_send(message)
            .map_err(|_| "网络发送拥堵，请重新连接".into())
    }
    fn event(&self, event: serde_json::Value) {
        let _ = self.app.emit("device-event", event);
    }
    fn begin(&mut self) -> Result<(), String> {
        self.response_expected.store(false, Ordering::Relaxed);
        self.encoder.reset()?;
        let epoch = self
            .flags
            .epoch
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        self.suppressed.store(true, Ordering::Relaxed);
        if let Ok(mut q) = self.playback.lock() {
            q.clear();
        }
        self.send(super::Outgoing::Begin(epoch))?;
        self.flags.listening.store(true, Ordering::Relaxed);
        self.event(json!({"type":"voice_start"}));
        Ok(())
    }
    fn encode(&mut self, samples: &[f32]) -> Result<(), String> {
        for packet in self.encoder.push(samples)? {
            self.send(super::Outgoing::Audio(
                self.flags.epoch.load(Ordering::Relaxed),
                packet,
            ))?;
        }
        Ok(())
    }
    fn end(&mut self) -> Result<(), String> {
        if !self.flags.listening.load(Ordering::Relaxed) {
            return Ok(());
        }
        let epoch = self.flags.epoch.load(Ordering::Relaxed);
        if let Some(tail) = self.encoder.finish()? {
            self.send(super::Outgoing::Audio(epoch, tail))?;
        }
        self.response_expected.store(true, Ordering::Relaxed);
        self.send(super::Outgoing::End(epoch))?;
        self.flags.listening.store(false, Ordering::Relaxed);
        self.event(json!({"type":"voice_end"}));
        Ok(())
    }
    fn cancel(&mut self) -> Result<(), String> {
        self.response_expected.store(false, Ordering::Relaxed);
        self.flags.listening.store(false, Ordering::Relaxed);
        self.suppressed.store(true, Ordering::Relaxed);
        let epoch = self
            .flags
            .epoch
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        if let Ok(mut q) = self.playback.lock() {
            q.clear();
        }
        self.endpoint.reset();
        self.pre_roll.clear();
        self.encoder.reset()?;
        self.send(super::Outgoing::Abort(epoch))?;
        self.event(json!({"type":"voice_cancelled"}));
        Ok(())
    }
    fn control(&mut self, control: Control) -> Result<(), String> {
        match control {
            Control::Call(enabled) => {
                self.flags.call.store(false, Ordering::Relaxed);
                self.cancel()?;
                self.flags.generation.fetch_add(1, Ordering::Relaxed);
                self.voice = VoiceProcessor::new();
                self.flags.call.store(enabled, Ordering::Relaxed);
                self.event(json!({"type":"call_state","enabled":enabled}));
            }
            Control::Start => {
                if self.flags.call.load(Ordering::Relaxed) {
                    return Err("通话中无需手动录音".into());
                }
                self.endpoint.reset();
                self.pre_roll.clear();
                self.begin()?;
            }
            Control::Stop => self.end()?,
            Control::Abort => self.cancel()?,
            Control::Text(text) => {
                self.cancel()?;
                self.response_expected.store(true, Ordering::Relaxed);
                self.send(super::Outgoing::Text(
                    self.flags.epoch.load(Ordering::Relaxed),
                    text,
                ))?;
            }
        }
        Ok(())
    }
    fn capture(&mut self, frame: &[f32; FRAME_SAMPLES], delay_ms: u32) -> Result<(), String> {
        if !self.flags.call.load(Ordering::Relaxed) {
            if self.flags.listening.load(Ordering::Relaxed) {
                self.encode(frame)?;
            }
            return Ok(());
        }
        let (clean, voiced) = self.voice.capture(frame, delay_ms)?;
        let was_active = self.endpoint.active();
        let decision = self
            .endpoint
            .push(voiced, self.silence_ms.load(Ordering::Relaxed));
        if was_active {
            self.encode(&clean)?;
        } else {
            self.pre_roll.extend(clean);
            while self.pre_roll.len() > PRE_ROLL_SAMPLES {
                self.pre_roll.pop_front();
            }
        }
        match decision {
            Decision::Start => {
                self.begin()?;
                let lead: Vec<f32> = self.pre_roll.drain(..).collect();
                self.encode(&lead)?;
            }
            Decision::Stop => self.end()?,
            _ => {}
        }
        Ok(())
    }
}

pub fn start(
    app: AppHandle,
    outgoing: Sender<super::Outgoing>,
    input: String,
    output: String,
    initial_volume: u32,
    initial_silence: u32,
) -> Result<Audio, String> {
    let listening = Arc::new(AtomicBool::new(false));
    let running = Arc::new(AtomicBool::new(true));
    let call_enabled = Arc::new(AtomicBool::new(false));
    let epoch = Arc::new(AtomicU32::new(0));
    let flags = Flags {
        listening: listening.clone(),
        running: running.clone(),
        call: call_enabled.clone(),
        epoch: epoch.clone(),
        generation: Arc::new(AtomicU32::new(0)),
    };
    let volume = Arc::new(AtomicU32::new(initial_volume));
    let silence_ms = Arc::new(AtomicU32::new(initial_silence));
    let suppressed = Arc::new(AtomicBool::new(true));
    let response_expected = Arc::new(AtomicBool::new(false));
    let playback = Arc::new(Mutex::new(VecDeque::new()));
    let (incoming, rx) = mpsc::sync_channel::<Vec<u8>>(128);
    let (controls, control_rx) = mpsc::sync_channel::<Request>(16);
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let (queue, vol, suppress, pause) = (
        playback.clone(),
        volume.clone(),
        suppressed.clone(),
        silence_ms.clone(),
    );
    let expected = response_expected.clone();
    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let mic = select_device(true, &input)?;
            let speaker = select_device(false, &output)?;
            let mic_config = mic.default_input_config().map_err(|e| e.to_string())?;
            let speaker_config = speaker.default_output_config().map_err(|e| e.to_string())?;
            let in_rate = mic_config.sample_rate().0;
            let out_rate = speaker_config.sample_rate().0;
            let (pcm_tx, pcm_rx) = mpsc::sync_channel::<Capture>(32);
            let (render_tx, render_rx) = mpsc::sync_channel::<Render>(32);
            let input = match mic_config.sample_format() {
                cpal::SampleFormat::F32 => input_stream::<f32>(
                    &mic,
                    &mic_config.into(),
                    flags.clone(),
                    pcm_tx,
                    app.clone(),
                ),
                cpal::SampleFormat::I16 => input_stream::<i16>(
                    &mic,
                    &mic_config.into(),
                    flags.clone(),
                    pcm_tx,
                    app.clone(),
                ),
                cpal::SampleFormat::U16 => input_stream::<u16>(
                    &mic,
                    &mic_config.into(),
                    flags.clone(),
                    pcm_tx,
                    app.clone(),
                ),
                _ => Err("不支持的麦克风采样格式".into()),
            }?;
            let output = match speaker_config.sample_format() {
                cpal::SampleFormat::F32 => output_stream::<f32>(
                    &speaker,
                    &speaker_config.into(),
                    queue.clone(),
                    vol,
                    flags.clone(),
                    render_tx,
                    app.clone(),
                ),
                cpal::SampleFormat::I16 => output_stream::<i16>(
                    &speaker,
                    &speaker_config.into(),
                    queue.clone(),
                    vol,
                    flags.clone(),
                    render_tx,
                    app.clone(),
                ),
                cpal::SampleFormat::U16 => output_stream::<u16>(
                    &speaker,
                    &speaker_config.into(),
                    queue.clone(),
                    vol,
                    flags.clone(),
                    render_tx,
                    app.clone(),
                ),
                _ => Err("不支持的扬声器采样格式".into()),
            }?;
            let mut worker = Worker {
                app: app.clone(),
                outgoing,
                flags: flags.clone(),
                suppressed: suppress,
                response_expected: expected,
                playback: queue.clone(),
                silence_ms: pause,
                voice: VoiceProcessor::new(),
                endpoint: Endpoint::default(),
                encoder: TurnEncoder::new()?,
                pre_roll: VecDeque::with_capacity(PRE_ROLL_SAMPLES),
            };
            let mut decoder =
                opus::Decoder::new(48000, opus::Channels::Mono).map_err(|e| e.to_string())?;
            let mut capture_resampler = Resampler::new(in_rate, 16000);
            let mut reference_resampler = Resampler::new(out_rate, 16000);
            let mut playback_resampler = Resampler::new(48000, out_rate);
            let mut capture_buffer = VecDeque::new();
            let mut reference_buffer = VecDeque::new();
            let mut decoded = [0.0; 5760];
            let mut meter = Instant::now();
            let mut generation = 0;
            let mut last_epoch = 0;
            let mut render_delay = 10;
            input.play().map_err(|e| e.to_string())?;
            output.play().map_err(|e| e.to_string())?;
            let _ = ready_tx.send(Ok(()));
            while flags.running.load(Ordering::Relaxed) {
                while let Ok(request) = control_rx.try_recv() {
                    let result = worker.control(request.control);
                    let _ = request.reply.send(result.clone());
                    result?;
                }
                let current_generation = flags.generation.load(Ordering::Relaxed);
                if current_generation != generation {
                    generation = current_generation;
                    capture_buffer.clear();
                    reference_buffer.clear();
                    capture_resampler = Resampler::new(in_rate, 16000);
                    reference_resampler = Resampler::new(out_rate, 16000);
                }
                for _ in 0..32 {
                    let Ok(render) = render_rx.try_recv() else {
                        break;
                    };
                    if render.generation != generation || !flags.call.load(Ordering::Relaxed) {
                        continue;
                    }
                    render_delay = render.delay_ms;
                    reference_buffer.extend(reference_resampler.push(&render.samples));
                    while reference_buffer.len() >= FRAME_SAMPLES {
                        let mut frame = [0.0; FRAME_SAMPLES];
                        for s in &mut frame {
                            *s = reference_buffer.pop_front().unwrap();
                        }
                        worker.voice.render(&frame)?;
                    }
                }
                for _ in 0..32 {
                    let Ok(capture) = pcm_rx.try_recv() else {
                        break;
                    };
                    if capture.generation != generation {
                        continue;
                    }
                    let call = flags.call.load(Ordering::Relaxed);
                    if !call
                        && (!flags.listening.load(Ordering::Relaxed)
                            || capture.epoch != flags.epoch.load(Ordering::Relaxed))
                    {
                        continue;
                    }
                    if !call && last_epoch != capture.epoch {
                        capture_buffer.clear();
                        capture_resampler = Resampler::new(in_rate, 16000);
                    }
                    last_epoch = capture.epoch;
                    if meter.elapsed() > Duration::from_millis(100) {
                        let level = (capture.samples.iter().map(|s| s * s).sum::<f32>()
                            / capture.samples.len().max(1) as f32)
                            .sqrt();
                        let _ = app.emit("audio-level", level);
                        meter = Instant::now();
                    }
                    let delay = render_delay
                        + capture.delay_ms
                        + capture.captured.elapsed().as_millis().min(200) as u32;
                    capture_buffer.extend(capture_resampler.push(&capture.samples));
                    while capture_buffer.len() >= FRAME_SAMPLES {
                        let mut frame = [0.0; FRAME_SAMPLES];
                        for s in &mut frame {
                            *s = capture_buffer.pop_front().unwrap();
                        }
                        worker.capture(&frame, delay)?;
                    }
                }
                // Decode a bounded number of packets before checking capture/control again.
                for _ in 0..4 {
                    let packet = if let Ok(packet) = rx.try_recv() {
                        packet
                    } else {
                        break;
                    };
                    if worker.suppressed.load(Ordering::Relaxed) {
                        continue;
                    }
                    let len = decoder
                        .decode_float(&packet, &mut decoded, false)
                        .map_err(|e| e.to_string())?;
                    let samples = playback_resampler.push(&decoded[..len]);
                    if let Ok(mut q) = queue.lock() {
                        if worker.suppressed.load(Ordering::Relaxed) {
                            continue;
                        }
                        if q.len() + samples.len() > out_rate as usize * 10 {
                            return Err("播放缓冲区已满，请重新连接".into());
                        }
                        q.extend(samples);
                    }
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Ok(())
        })();
        if let Err(error) = result {
            let _ = ready_tx.try_send(Err(error.clone()));
            let _ = app.emit(
                "device-event",
                json!({"type":"audio_error","message":error}),
            );
        }
        flags.call.store(false, Ordering::Relaxed);
        flags.listening.store(false, Ordering::Relaxed);
        flags.running.store(false, Ordering::Relaxed);
    });
    match ready_rx.recv_timeout(Duration::from_secs(15)) {
        Ok(Ok(())) => Ok(Audio {
            listening,
            running,
            volume,
            epoch,
            suppressed,
            response_expected,
            call_enabled,
            silence_ms,
            playback,
            incoming,
            controls,
        }),
        Ok(Err(e)) => {
            running.store(false, Ordering::Relaxed);
            Err(e)
        }
        Err(_) => {
            running.store(false, Ordering::Relaxed);
            Err("音频设备初始化超时".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resampling_keeps_phase_between_blocks() {
        let input: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.03).sin()).collect();
        let expected = Resampler::new(48000, 16000).push(&input);
        let mut r = Resampler::new(48000, 16000);
        let actual: Vec<f32> = input.chunks(137).flat_map(|c| r.push(c)).collect();
        assert_eq!(expected, actual);
        assert!((actual.len() as i32 - 1600).abs() <= 1);
    }
    #[test]
    fn opus_roundtrip_is_one_sixty_ms_frame() {
        let mut encoder =
            opus::Encoder::new(16000, opus::Channels::Mono, opus::Application::Voip).unwrap();
        let mut decoder = opus::Decoder::new(48000, opus::Channels::Mono).unwrap();
        let mut packet = [0; 1275];
        let size = encoder.encode_float(&[0.0; 960], &mut packet).unwrap();
        assert_eq!(
            decoder
                .decode_float(&packet[..size], &mut [0.0; 5760], false)
                .unwrap(),
            2880
        );
    }
}
