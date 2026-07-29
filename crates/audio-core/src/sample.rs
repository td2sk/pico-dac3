#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SampleQ31(i32);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GainU31(u32);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StereoFrame {
    pub left: SampleQ31,
    pub right: SampleQ31,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    Pcm16,
    Pcm24In32,
    Pcm32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum SampleRate {
    Hz44100 = 44_100,
    #[default]
    Hz48000 = 48_000,
    Hz88200 = 88_200,
    Hz96000 = 96_000,
}

impl SampleRate {
    pub const ALL: [Self; 4] = [Self::Hz44100, Self::Hz48000, Self::Hz88200, Self::Hz96000];

    pub const fn hz(self) -> u32 {
        self as u32
    }

    pub const fn from_hz(value: u32) -> Option<Self> {
        match value {
            44_100 => Some(Self::Hz44100),
            48_000 => Some(Self::Hz48000),
            88_200 => Some(Self::Hz88200),
            96_000 => Some(Self::Hz96000),
            _ => None,
        }
    }
}

impl SampleFormat {
    pub const fn bytes_per_frame(self) -> usize {
        match self {
            Self::Pcm16 => 4,
            Self::Pcm24In32 | Self::Pcm32 => 8,
        }
    }
}

impl SampleQ31 {
    pub const SILENCE: Self = Self(0);

    pub const fn from_pcm16(value: i16) -> Self {
        Self((value as i32) << 16)
    }

    pub const fn from_left_aligned_pcm24(value: i32) -> Self {
        Self(value)
    }

    pub const fn from_pcm32(value: i32) -> Self {
        Self(value)
    }

    pub const fn raw(self) -> i32 {
        self.0
    }

    pub const fn to_pcm16(self) -> i16 {
        (self.0 >> 16) as i16
    }

    pub const fn to_pcm24(self) -> i32 {
        self.0 >> 8
    }

    pub const fn to_i2s_word(self, format: SampleFormat) -> u32 {
        match format {
            SampleFormat::Pcm16 => (self.0 >> 16) as u32,
            SampleFormat::Pcm24In32 => (self.0 >> 8) as u32,
            SampleFormat::Pcm32 => self.0 as u32,
        }
    }

    pub fn apply_gain(self, gain: GainU31) -> Self {
        let value = (self.0 as i64 * gain.0 as i64) >> 31;
        Self(value.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
    }
}

impl GainU31 {
    pub const SILENCE: Self = Self(0);
    pub const UNITY: Self = Self(1 << 31);

    pub const fn from_raw(value: u32) -> Self {
        Self(if value > 1 << 31 { 1 << 31 } else { value })
    }

    pub fn compose(self, other: Self) -> Self {
        Self(((self.0 as u64 * other.0 as u64) >> 31) as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm16_round_trips() {
        for value in [i16::MIN, -12345, -1, 0, 1, 12345, i16::MAX] {
            assert_eq!(SampleQ31::from_pcm16(value).to_pcm16(), value);
        }
    }

    #[test]
    fn pcm24_is_left_aligned() {
        assert_eq!(
            SampleQ31::from_left_aligned_pcm24(0x1234_5600).to_pcm24(),
            0x123456
        );
        assert_eq!(
            SampleQ31::from_left_aligned_pcm24(i32::MIN).to_pcm24(),
            -0x800000
        );
    }

    #[test]
    fn gain_has_exact_unity_and_silence() {
        let input = SampleQ31::from_pcm32(0x4000_0000);
        assert_eq!(input.apply_gain(GainU31::UNITY), input);
        assert_eq!(input.apply_gain(GainU31::SILENCE), SampleQ31::SILENCE);
    }

    #[test]
    fn packs_right_aligned_i2s_words() {
        assert_eq!(
            SampleQ31::from_pcm32(0x1234_0000).to_i2s_word(SampleFormat::Pcm16),
            0x1234
        );
        assert_eq!(
            SampleQ31::from_pcm32(0x1234_5600).to_i2s_word(SampleFormat::Pcm24In32),
            0x123456
        );
        assert_eq!(
            SampleQ31::from_pcm32(i32::MIN).to_i2s_word(SampleFormat::Pcm16),
            0xffff_8000
        );
    }
}
