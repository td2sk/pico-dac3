#![cfg_attr(not(any(test, feature = "std")), no_std)]
#![deny(unsafe_code)]

mod controls;
mod engine;
mod fifo;
mod sample;

pub use controls::{Channel, Controls, Volume};
pub use engine::{AudioEngine, EngineState, FeedbackQ16, PacketError, StreamConfig};
pub use fifo::AudioFifo;
pub use sample::{GainU31, SampleFormat, SampleQ31, SampleRate, StereoFrame};

pub const MAX_FIFO_FRAMES: usize = 96_000 * 16 / 1_000;
