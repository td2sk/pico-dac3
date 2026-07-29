use crate::GainU31;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Master,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Volume(i16);

impl Volume {
    pub const MIN: Self = Self(-96 * 256);
    pub const MAX: Self = Self(0);
    pub const RESOLUTION: i16 = 256;

    pub const fn from_db_256(value: i16) -> Option<Self> {
        if value >= Self::MIN.0 && value <= Self::MAX.0 && value % 256 == 0 {
            Some(Self(value))
        } else {
            None
        }
    }

    pub const fn db_256(self) -> i16 {
        self.0
    }

    pub fn gain(self) -> GainU31 {
        GainU31::from_raw(DB_GAIN_U31[(-self.0 / 256) as usize])
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Controls {
    mute: [bool; 3],
    volume: [Volume; 3],
}

impl Controls {
    pub const fn new() -> Self {
        Self {
            mute: [false; 3],
            volume: [Volume::MAX; 3],
        }
    }

    const fn index(channel: Channel) -> usize {
        match channel {
            Channel::Master => 0,
            Channel::Left => 1,
            Channel::Right => 2,
        }
    }

    pub fn set_mute(&mut self, channel: Channel, value: bool) {
        self.mute[Self::index(channel)] = value;
    }

    pub fn mute(&self, channel: Channel) -> bool {
        self.mute[Self::index(channel)]
    }

    pub fn set_volume(&mut self, channel: Channel, value: Volume) {
        self.volume[Self::index(channel)] = value;
    }

    pub fn volume(&self, channel: Channel) -> Volume {
        self.volume[Self::index(channel)]
    }

    pub fn effective_gains(&self) -> (GainU31, GainU31) {
        let master = self.volume[0].gain();
        let left = if self.mute[0] || self.mute[1] {
            GainU31::SILENCE
        } else {
            master.compose(self.volume[1].gain())
        };
        let right = if self.mute[0] || self.mute[2] {
            GainU31::SILENCE
        } else {
            master.compose(self.volume[2].gain())
        };
        (left, right)
    }
}

impl Default for Controls {
    fn default() -> Self {
        Self::new()
    }
}

// UQ1.31 values for 10^(-dB/20), from 0 through -96 dB.
const DB_GAIN_U31: [u32; 97] = [
    2147483648, 1913946816, 1705806895, 1520301996, 1354970580, 1207618800, 1076291389, 959245710,
    854928639, 761955951, 679093957, 605243126, 539423504, 480761704, 428479319, 381882595,
    340353221, 303340128, 270352174, 240951628, 214748365, 191394682, 170580690, 152030200,
    135497058, 120761880, 107629139, 95924571, 85492864, 76195595, 67909396, 60524313, 53942350,
    48076170, 42847932, 38188260, 34035322, 30334013, 27035217, 24095163, 21474836, 19139468,
    17058069, 15203020, 13549706, 12076188, 10762914, 9592457, 8549286, 7619560, 6790940, 6052431,
    5394235, 4807617, 4284793, 3818826, 3403532, 3033401, 2703522, 2409516, 2147484, 1913947,
    1705807, 1520302, 1354971, 1207619, 1076291, 959246, 854929, 761956, 679094, 605243, 539424,
    480762, 428479, 381883, 340353, 303340, 270352, 240952, 214748, 191395, 170581, 152030, 135497,
    120762, 107629, 95925, 85493, 76196, 67909, 60524, 53942, 48076, 42848, 38188, 34035,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_volume_and_combines_mute() {
        assert!(Volume::from_db_256(-97 * 256).is_none());
        let mut controls = Controls::new();
        controls.set_mute(Channel::Master, true);
        assert_eq!(
            controls.effective_gains(),
            (GainU31::SILENCE, GainU31::SILENCE)
        );
    }
}
