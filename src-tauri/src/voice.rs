use sonora::{
    config::{EchoCanceller, HighPassFilter, NoiseSuppression},
    AudioProcessing, Config, StreamConfig,
};
use webrtc_vad::{SampleRate, Vad, VadMode};

pub const FRAME_SAMPLES: usize = 160;
pub const DEFAULT_SILENCE_MS: u32 = 700;
pub const PRE_ROLL_SAMPLES: usize = 3200;

pub fn validate_silence(ms: u32) -> Result<u32, String> {
    if (300..=2000).contains(&ms) {
        Ok(ms)
    } else {
        Err("停顿时长应在 300–2000 毫秒之间".into())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    None,
    Start,
    Continue,
    Stop,
}

/// Sample-clock endpointing: silence outside a turn never sends an empty utterance.
#[derive(Default)]
pub struct Endpoint {
    active: bool,
    onset_ms: u32,
    quiet_ms: u32,
    turn_ms: u32,
}
impl Endpoint {
    pub fn active(&self) -> bool {
        self.active
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn push(&mut self, voiced: bool, silence_ms: u32) -> Decision {
        if !self.active {
            self.onset_ms = if voiced { self.onset_ms + 10 } else { 0 };
            if self.onset_ms < 60 {
                return Decision::None;
            }
            self.active = true;
            self.quiet_ms = 0;
            self.turn_ms = self.onset_ms;
            return Decision::Start;
        }
        self.turn_ms += 10;
        self.quiet_ms = if voiced { 0 } else { self.quiet_ms + 10 };
        if self.quiet_ms >= silence_ms || self.turn_ms >= 45000 {
            self.reset();
            return Decision::Stop;
        }
        Decision::Continue
    }
}

/// Local echo cancellation + noise suppression, followed by WebRTC speech classification.
/// All streams are mono, 16 kHz, normalized f32, in exact 10 ms frames.
pub struct VoiceProcessor {
    apm: AudioProcessing,
    vad: Vad,
}
impl Default for VoiceProcessor {
    fn default() -> Self {
        Self::new()
    }
}
impl VoiceProcessor {
    pub fn new() -> Self {
        let config = Config {
            echo_canceller: Some(EchoCanceller::default()),
            high_pass_filter: Some(HighPassFilter::default()),
            noise_suppression: Some(NoiseSuppression::default()),
            ..Default::default()
        };
        Self {
            apm: AudioProcessing::builder()
                .config(config)
                .capture_config(StreamConfig::new(16000, 1))
                .render_config(StreamConfig::new(16000, 1))
                .build(),
            vad: Vad::new_with_rate_and_mode(SampleRate::Rate16kHz, VadMode::Aggressive),
        }
    }
    pub fn render(&mut self, samples: &[f32; FRAME_SAMPLES]) -> Result<(), String> {
        self.apm
            .process_render_f32(&[samples], &mut [&mut [0.0; FRAME_SAMPLES]])
            .map_err(|e| format!("回声参考处理失败：{e:?}"))
    }
    pub fn capture(
        &mut self,
        samples: &[f32; FRAME_SAMPLES],
        delay_ms: u32,
    ) -> Result<([f32; FRAME_SAMPLES], bool), String> {
        let mut clean = [0.0; FRAME_SAMPLES];
        self.apm
            .set_stream_delay_ms(delay_ms.min(500) as i32)
            .map_err(|e| format!("音频延迟设置失败：{e:?}"))?;
        self.apm
            .process_capture_f32(&[samples], &mut [&mut clean])
            .map_err(|e| format!("麦克风降噪失败：{e:?}"))?;
        let pcm = clean.map(|s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
        let speech = self
            .vad
            .is_voice_segment(&pcm)
            .map_err(|_| "语音检测失败")?;
        let energy = clean.iter().map(|s| s * s).sum::<f32>() / FRAME_SAMPLES as f32;
        Ok((clean, speech && energy > 0.000004))
    }
}

pub struct TurnEncoder {
    encoder: opus::Encoder,
    pending: Vec<f32>,
    packet: [u8; 1275],
}
impl TurnEncoder {
    pub fn new() -> Result<Self, String> {
        let mut encoder = opus::Encoder::new(16000, opus::Channels::Mono, opus::Application::Voip)
            .map_err(|e| e.to_string())?;
        encoder
            .set_bitrate(opus::Bitrate::Bits(24000))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            encoder,
            pending: Vec::with_capacity(960),
            packet: [0; 1275],
        })
    }
    pub fn reset(&mut self) -> Result<(), String> {
        self.pending.clear();
        self.encoder.reset_state().map_err(|e| e.to_string())
    }
    pub fn push(&mut self, samples: &[f32]) -> Result<Vec<Vec<u8>>, String> {
        let mut packets = Vec::new();
        for chunk in samples.chunks(160) {
            self.pending.extend_from_slice(chunk);
            while self.pending.len() >= 960 {
                let size = self
                    .encoder
                    .encode_float(&self.pending[..960], &mut self.packet)
                    .map_err(|e| e.to_string())?;
                packets.push(self.packet[..size].to_vec());
                self.pending.drain(..960);
            }
        }
        Ok(packets)
    }
    pub fn finish(&mut self) -> Result<Option<Vec<u8>>, String> {
        if self.pending.is_empty() {
            return Ok(None);
        }
        self.pending.resize(960, 0.0);
        let size = self
            .encoder
            .encode_float(&self.pending, &mut self.packet)
            .map_err(|e| e.to_string())?;
        self.pending.clear();
        Ok(Some(self.packet[..size].to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture_speech() -> Vec<f32> {
        let ogg = include_bytes!("../../tests/fixtures/upstream-activation.ogg");
        let mut offset = 0;
        let mut packet = Vec::new();
        let mut audio = Vec::new();
        let mut decoder = opus::Decoder::new(16000, opus::Channels::Mono).unwrap();
        while offset < ogg.len() {
            assert_eq!(&ogg[offset..offset + 4], b"OggS");
            let segments = ogg[offset + 26] as usize;
            let laces = &ogg[offset + 27..offset + 27 + segments];
            let mut payload = offset + 27 + segments;
            for &size in laces {
                packet.extend_from_slice(&ogg[payload..payload + size as usize]);
                payload += size as usize;
                if size < 255 {
                    if !packet.starts_with(b"OpusHead") && !packet.starts_with(b"OpusTags") {
                        let mut pcm = [0.0; 1920];
                        let n = decoder.decode_float(&packet, &mut pcm, false).unwrap();
                        audio.extend_from_slice(&pcm[..n]);
                    }
                    packet.clear();
                }
            }
            offset = payload;
        }
        audio
    }
    #[test]
    fn real_upstream_voice_is_detected_then_submitted_after_the_pause() {
        let mut audio = fixture_speech();
        audio.extend_from_slice(&[0.0; 32000]);
        let mut processor = VoiceProcessor::new();
        let mut gate = Endpoint::default();
        let mut starts = 0;
        let mut stops = 0;
        for chunk in audio.chunks_exact(FRAME_SAMPLES) {
            processor.render(&[0.0; FRAME_SAMPLES]).unwrap();
            let (_, voiced) = processor.capture(chunk.try_into().unwrap(), 20).unwrap();
            match gate.push(voiced, 700) {
                Decision::Start => starts += 1,
                Decision::Stop => stops += 1,
                _ => {}
            }
        }
        assert!(starts > 0, "real spoken audio was not detected");
        assert_eq!(starts, stops);
        assert!(!gate.active());
    }
    #[test]
    fn echo_is_reduced_and_near_end_speech_can_still_interrupt() {
        let near = fixture_speech();
        let total = 64000 + near.len() + 32000;
        let mut seed = 12345u32;
        let far: Vec<f32> = (0..total)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                ((seed >> 8) as f32 / 16777216.0 - 0.5) * 0.4
            })
            .collect();
        let mut processor = VoiceProcessor::new();
        let mut echo_input = 0.0f64;
        let mut echo_output = 0.0f64;
        let mut near_frames = 0;
        for offset in (0..total - FRAME_SAMPLES).step_by(FRAME_SAMPLES) {
            let render: &[f32; FRAME_SAMPLES] =
                far[offset..offset + FRAME_SAMPLES].try_into().unwrap();
            let mut mic = [0.0; FRAME_SAMPLES];
            for (i, s) in mic.iter_mut().enumerate() {
                let n = offset + i;
                *s = if n >= 640 { far[n - 640] * 0.4 } else { 0.0 };
                if n >= 64000 && n < 64000 + near.len() {
                    *s += near[n - 64000] * 0.8;
                }
            }
            processor.render(render).unwrap();
            let (out, voiced) = processor.capture(&mic, 40).unwrap();
            if (48000..64000).contains(&offset) {
                echo_input += mic.iter().map(|s| (*s as f64).powi(2)).sum::<f64>();
                echo_output += out.iter().map(|s| (*s as f64).powi(2)).sum::<f64>();
            }
            if offset >= 64000 && offset < 64000 + near.len() && voiced {
                near_frames += 1;
            }
        }
        assert!(
            echo_output < echo_input * 0.3,
            "echo power ratio {}",
            echo_output / echo_input
        );
        assert!(
            near_frames >= 6,
            "near-end voice was suppressed during echo cancellation"
        );
    }
    #[test]
    fn silence_never_submits_empty_turns_and_clicks_do_not_start() {
        let mut e = Endpoint::default();
        for _ in 0..500 {
            assert_eq!(e.push(false, 700), Decision::None);
        }
        for _ in 0..5 {
            assert_eq!(e.push(true, 700), Decision::None);
        }
        assert_eq!(e.push(false, 700), Decision::None);
        assert!(!e.active());
    }
    #[test]
    fn configurable_pause_resets_when_speech_resumes() {
        for pause in [300, 700, 2000] {
            let mut e = Endpoint::default();
            for _ in 0..5 {
                e.push(true, pause);
            }
            assert_eq!(e.push(true, pause), Decision::Start);
            for _ in 0..(pause / 10 - 1) {
                assert_eq!(e.push(false, pause), Decision::Continue);
            }
            assert_eq!(e.push(true, pause), Decision::Continue);
            for _ in 0..(pause / 10 - 1) {
                assert_eq!(e.push(false, pause), Decision::Continue);
            }
            assert_eq!(e.push(false, pause), Decision::Stop);
            for _ in 0..30 {
                assert_eq!(e.push(false, pause), Decision::None);
            }
        }
    }
    #[test]
    fn pause_can_be_adjusted_during_a_turn() {
        let mut e = Endpoint::default();
        for _ in 0..6 {
            e.push(true, 2000);
        }
        for _ in 0..40 {
            e.push(false, 2000);
        }
        assert_eq!(e.push(false, 300), Decision::Stop);
        assert!(validate_silence(299).is_err());
        assert!(validate_silence(2001).is_err());
    }
    #[test]
    fn a_new_utterance_can_interrupt_while_waiting_for_a_reply() {
        let mut e = Endpoint::default();
        for _ in 0..6 {
            e.push(true, 300);
        }
        for _ in 0..30 {
            e.push(false, 300);
        }
        for _ in 0..5 {
            assert_eq!(e.push(true, 300), Decision::None);
        }
        assert_eq!(e.push(true, 300), Decision::Start);
    }
    #[test]
    fn trailing_audio_is_flushed_to_a_full_opus_frame() {
        let mut encoder = TurnEncoder::new().unwrap();
        assert!(encoder.push(&[0.1; 160]).unwrap().is_empty());
        let packet = encoder.finish().unwrap().unwrap();
        let mut decoder = opus::Decoder::new(16000, opus::Channels::Mono).unwrap();
        assert_eq!(
            decoder
                .decode_float(&packet, &mut [0.0; 1920], false)
                .unwrap(),
            960
        );
        assert!(encoder.finish().unwrap().is_none());
    }
    #[test]
    fn echo_pipeline_silence_stays_finite_and_does_not_trigger_voice() {
        let mut v = VoiceProcessor::new();
        for _ in 0..100 {
            v.render(&[0.0; 160]).unwrap();
            let (out, speech) = v.capture(&[0.0; 160], 20).unwrap();
            assert!(!speech);
            assert!(out.iter().all(|s| s.is_finite()));
        }
    }
}
