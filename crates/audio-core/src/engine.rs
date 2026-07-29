use crate::{
    AudioFifo, Channel, Controls, MAX_FIFO_FRAMES, SampleFormat, SampleQ31, SampleRate,
    StereoFrame, Volume,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamConfig {
    pub rate: SampleRate,
    pub format: SampleFormat,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EngineState {
    #[default]
    Disabled,
    Priming,
    Running,
    Recovering,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketError {
    NotStreaming,
    Misaligned,
    Overrun,
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FeedbackQ16(u32);

impl FeedbackQ16 {
    pub const fn from_rate_hz(rate: u32) -> Self {
        Self(((rate as u64) << 16).div_ceil(1_000) as u32)
    }

    pub const fn raw(self) -> u32 {
        self.0
    }

    pub const fn to_usb_bytes(self) -> [u8; 4] {
        self.0.to_le_bytes()
    }
}

pub struct AudioEngine {
    config: Option<StreamConfig>,
    state: EngineState,
    fifo: AudioFifo<MAX_FIFO_FRAMES>,
    controls: Controls,
    target_frames: usize,
    nominal_rate_q16: i64,
    filtered_fill_q16: i64,
    underruns: u32,
    overruns: u32,
}

impl AudioEngine {
    pub const fn new() -> Self {
        Self {
            config: None,
            state: EngineState::Disabled,
            fifo: AudioFifo::new(),
            controls: Controls::new(),
            target_frames: 384,
            nominal_rate_q16: (48_000_i64 << 16) / 1_000,
            filtered_fill_q16: 384 << 16,
            underruns: 0,
            overruns: 0,
        }
    }

    pub fn start(&mut self, config: StreamConfig, actual_i2s_rate_hz: u32) {
        let capacity = config.rate.hz() as usize * 16 / 1_000;
        self.target_frames = capacity / 2;
        self.fifo.set_capacity(capacity);
        self.nominal_rate_q16 = ((actual_i2s_rate_hz as i64) << 16) / 1_000;
        self.filtered_fill_q16 = (self.target_frames as i64) << 16;
        self.config = Some(config);
        self.state = EngineState::Priming;
    }

    pub fn stop(&mut self) {
        self.config = None;
        self.fifo.clear();
        self.state = EngineState::Disabled;
    }

    pub const fn state(&self) -> EngineState {
        self.state
    }

    pub const fn fill_frames(&self) -> usize {
        self.fifo.len()
    }

    pub fn set_mute(&mut self, channel: Channel, value: bool) {
        self.controls.set_mute(channel, value);
    }

    pub fn set_volume(&mut self, channel: Channel, value: Volume) {
        self.controls.set_volume(channel, value);
    }

    pub fn feedback(&mut self) -> FeedbackQ16 {
        let measured = (self.fifo.len() as i64) << 16;
        self.filtered_fill_q16 += (measured - self.filtered_fill_q16) / 100;
        let error = self.filtered_fill_q16 - ((self.target_frames as i64) << 16);
        let correction = (error >> 8).clamp(-(200_i64 << 16), 200_i64 << 16);
        FeedbackQ16((self.nominal_rate_q16 - correction).max(0) as u32)
    }

    pub fn push_usb_packet(&mut self, packet: &[u8]) -> Result<usize, PacketError> {
        let config = self.config.ok_or(PacketError::NotStreaming)?;
        let stride = config.format.bytes_per_frame();
        if packet.len() % stride != 0 {
            return Err(PacketError::Misaligned);
        }
        let count = packet.len() / stride;
        if count > self.fifo.free() {
            self.overruns = self.overruns.saturating_add(1);
            self.fifo.clear();
            self.state = EngineState::Recovering;
            return Err(PacketError::Overrun);
        }
        for bytes in packet.chunks_exact(stride) {
            let frame = match config.format {
                SampleFormat::Pcm16 => StereoFrame {
                    left: SampleQ31::from_pcm16(i16::from_le_bytes([bytes[0], bytes[1]])),
                    right: SampleQ31::from_pcm16(i16::from_le_bytes([bytes[2], bytes[3]])),
                },
                SampleFormat::Pcm24In32 => StereoFrame {
                    left: SampleQ31::from_left_aligned_pcm24(i32::from_le_bytes(
                        bytes[0..4].try_into().unwrap(),
                    )),
                    right: SampleQ31::from_left_aligned_pcm24(i32::from_le_bytes(
                        bytes[4..8].try_into().unwrap(),
                    )),
                },
                SampleFormat::Pcm32 => StereoFrame {
                    left: SampleQ31::from_pcm32(i32::from_le_bytes(
                        bytes[0..4].try_into().unwrap(),
                    )),
                    right: SampleQ31::from_pcm32(i32::from_le_bytes(
                        bytes[4..8].try_into().unwrap(),
                    )),
                },
            };
            let _ = self.fifo.push(frame);
        }
        if matches!(self.state, EngineState::Priming | EngineState::Recovering)
            && self.fifo.len() >= self.target_frames
        {
            self.state = EngineState::Running;
        }
        Ok(count)
    }

    pub fn render(&mut self, output: &mut [StereoFrame]) -> usize {
        if self.state == EngineState::Recovering
            && self.fifo.len() * 100 >= self.fifo.capacity() * 40
        {
            self.state = EngineState::Running;
        }
        if self.state != EngineState::Running {
            output.fill(StereoFrame::default());
            return 0;
        }
        if self.fifo.len() * 100 <= self.fifo.capacity() * 16 {
            output.fill(StereoFrame::default());
            self.state = EngineState::Recovering;
            self.underruns = self.underruns.saturating_add(1);
            return 0;
        }
        let (left_gain, right_gain) = self.controls.effective_gains();
        let mut rendered = 0;
        for slot in output.iter_mut() {
            if let Some(frame) = self.fifo.pop() {
                *slot = StereoFrame {
                    left: frame.left.apply_gain(left_gain),
                    right: frame.right.apply_gain(right_gain),
                };
                rendered += 1;
            } else {
                *slot = StereoFrame::default();
            }
        }
        if rendered != output.len() {
            self.underruns = self.underruns.saturating_add(1);
            self.fifo.clear();
            self.state = EngineState::Recovering;
        }
        rendered
    }
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_pcm16_and_primes() {
        let mut engine = AudioEngine::new();
        engine.start(
            StreamConfig {
                rate: SampleRate::Hz48000,
                format: SampleFormat::Pcm16,
            },
            48_000,
        );
        let packet = [0x00, 0x40, 0x00, 0xc0].repeat(384);
        assert_eq!(engine.push_usb_packet(&packet), Ok(384));
        assert_eq!(engine.state(), EngineState::Running);
        let mut output = [StereoFrame::default(); 1];
        assert_eq!(engine.render(&mut output), 1);
        assert_eq!(output[0].left.to_pcm16(), 0x4000);
        assert_eq!(output[0].right.to_pcm16(), -0x4000);
    }

    #[test]
    fn feedback_is_full_speed_frames_per_millisecond_q16() {
        assert_eq!(FeedbackQ16::from_rate_hz(48_000).raw(), 48 << 16);
        assert_eq!(
            FeedbackQ16::from_rate_hz(44_100).raw(),
            ((44_100_u64 << 16).div_ceil(1_000)) as u32
        );
    }

    #[test]
    fn stalls_at_low_watermark_and_recovers_at_forty_percent() {
        let mut engine = AudioEngine::new();
        engine.start(
            StreamConfig {
                rate: SampleRate::Hz48000,
                format: SampleFormat::Pcm16,
            },
            48_000,
        );
        let frame = [0_u8; 4];
        for _ in 0..384 {
            engine.push_usb_packet(&frame).unwrap();
        }
        let mut output = [StereoFrame::default(); 269];
        assert_eq!(engine.render(&mut output), 269);
        assert_eq!(engine.render(&mut output[..1]), 0);
        assert_eq!(engine.state(), EngineState::Recovering);
        for _ in 0..288 {
            engine.push_usb_packet(&frame).unwrap();
        }
        assert_eq!(engine.render(&mut output[..1]), 1);
        assert_eq!(engine.state(), EngineState::Running);
    }
}
