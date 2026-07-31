use audio_core::{AudioEngine, EngineState, SampleFormat, StreamConfig};
use uac2_speaker::{ControlChange, Uac2Event};

use crate::i2s::I2s;

pub struct StreamController {
    desired: Option<StreamConfig>,
    reconfigure_i2s: bool,
}

impl StreamController {
    pub const fn new() -> Self {
        Self {
            desired: None,
            reconfigure_i2s: false,
        }
    }

    pub fn handle_event(
        &mut self,
        event: Uac2Event,
        current_format: Option<SampleFormat>,
        audio: &mut AudioEngine,
    ) {
        match event {
            Uac2Event::StreamStarted { rate, format } => {
                let config = StreamConfig { rate, format };
                self.desired = Some(config);
                audio.start(config, rate.hz());
                self.reconfigure_i2s = true;
            }
            Uac2Event::StreamStopped => {
                self.desired = None;
                audio.stop();
                self.reconfigure_i2s = true;
            }
            Uac2Event::ControlChanged(change) => match change {
                ControlChange::SampleRate(rate) => {
                    self.desired = current_format.map(|format| StreamConfig { rate, format });
                    if let Some(config) = self.desired {
                        audio.start(config, rate.hz());
                    } else {
                        audio.stop();
                    }
                    self.reconfigure_i2s = true;
                }
                ControlChange::Mute { channel, value } => audio.set_mute(channel, value),
                ControlChange::Volume { channel, value } => audio.set_volume(channel, value),
            },
        }
    }

    pub fn reconcile(&mut self, audio: &mut AudioEngine, i2s: &mut I2s) {
        if self.reconfigure_i2s || i2s.active_config() != self.desired {
            i2s.stop();
            self.reconfigure_i2s = false;
        }

        if i2s.is_idle()
            && audio.state() == EngineState::Running
            && let Some(config) = self.desired
        {
            i2s.start(audio, config);
        }
    }
}
