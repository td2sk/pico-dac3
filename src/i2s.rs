use audio_core::{AudioEngine, SampleFormat, StereoFrame, StreamConfig};
use embedded_dma::ReadBuffer;

use crate::hal::{
    self,
    dma::{Channel, double_buffer},
    pio::{
        Buffers, PIO, PIOBuilder, PinDir, Running, Rx, SM0, ShiftDirection, StateMachine, Tx,
        UninitStateMachine,
    },
};

pub const MAX_DMA_FRAMES: usize = 96;
pub const MAX_DMA_WORDS: usize = MAX_DMA_FRAMES * 2;

type Pio = PIO<hal::pac::PIO0>;
type Sm = (hal::pac::PIO0, SM0);
type UninitSm = UninitStateMachine<Sm>;
type RunningSm = StateMachine<Sm, Running>;
type FirstChannel = Channel<hal::dma::CH0>;
type SecondChannel = Channel<hal::dma::CH1>;
type QueuedTransfer = double_buffer::Transfer<
    FirstChannel,
    SecondChannel,
    DmaBuffer,
    Tx<Sm>,
    double_buffer::ReadNext<DmaBuffer>,
>;
type LastTransfer = double_buffer::Transfer<FirstChannel, SecondChannel, DmaBuffer, Tx<Sm>, ()>;

struct DmaBuffer {
    storage: &'static mut [u32; MAX_DMA_WORDS],
    frames: usize,
}

impl DmaBuffer {
    const fn new(storage: &'static mut [u32; MAX_DMA_WORDS]) -> Self {
        Self { storage, frames: 0 }
    }

    fn configure(&mut self, frames: usize) {
        assert!(frames <= MAX_DMA_FRAMES);
        self.frames = frames;
    }

    fn words_mut(&mut self) -> &mut [u32] {
        &mut self.storage[..self.frames * 2]
    }
}

// Safety: `storage` has static lifetime, and `frames` is not mutated while the
// buffer is owned by a DMA transfer. The reported prefix therefore remains
// valid and stable for the complete transfer.
unsafe impl ReadBuffer for DmaBuffer {
    type Word = u32;

    unsafe fn read_buffer(&self) -> (*const Self::Word, usize) {
        (self.storage.as_ptr(), self.frames * 2)
    }
}

enum State {
    Idle {
        pio: Pio,
        sm: UninitSm,
        channels: (FirstChannel, SecondChannel),
        buffer_a: DmaBuffer,
        buffer_b: DmaBuffer,
    },
    Running {
        pio: Pio,
        sm: RunningSm,
        rx: Rx<Sm>,
        transfer: QueuedTransfer,
        config: StreamConfig,
        stop_requested: bool,
    },
    Draining {
        pio: Pio,
        sm: RunningSm,
        rx: Rx<Sm>,
        transfer: LastTransfer,
        completed_buffer: DmaBuffer,
        config: StreamConfig,
    },
}

pub struct I2s {
    state: Option<State>,
    system_clock_hz: u32,
}

impl I2s {
    pub fn new(
        pio: Pio,
        sm: UninitSm,
        channels: (FirstChannel, SecondChannel),
        buffer_a: &'static mut [u32; MAX_DMA_WORDS],
        buffer_b: &'static mut [u32; MAX_DMA_WORDS],
        system_clock_hz: u32,
    ) -> Self {
        Self {
            state: Some(State::Idle {
                pio,
                sm,
                channels,
                buffer_a: DmaBuffer::new(buffer_a),
                buffer_b: DmaBuffer::new(buffer_b),
            }),
            system_clock_hz,
        }
    }

    pub const fn active_config(&self) -> Option<StreamConfig> {
        match &self.state {
            Some(State::Running { config, .. } | State::Draining { config, .. }) => Some(*config),
            _ => None,
        }
    }

    pub const fn is_idle(&self) -> bool {
        matches!(self.state, Some(State::Idle { .. }))
    }

    pub fn start(&mut self, audio: &mut AudioEngine, config: StreamConfig) {
        let state = self.state.take().expect("I2S state is always present");
        let State::Idle {
            mut pio,
            sm,
            channels,
            mut buffer_a,
            mut buffer_b,
        } = state
        else {
            self.state = Some(state);
            return;
        };

        let cycles_per_frame = cycles_per_frame(config.format);
        let divider_256 = ((self.system_clock_hz as u64 * 256)
            / (config.rate.hz() as u64 * cycles_per_frame as u64)) as u32;
        let actual_rate_hz = ((self.system_clock_hz as u64 * 256)
            / (divider_256 as u64 * cycles_per_frame as u64)) as u32;
        audio.set_actual_output_rate(actual_rate_hz);
        let installed = pio
            .install(&program(config.format))
            .expect("I2S PIO program fits");
        let (mut sm, rx, tx) = PIOBuilder::from_installed_program(installed)
            .out_pins(22, 1)
            .side_set_pin_base(20)
            .clock_divisor_fixed_point((divider_256 / 256) as u16, divider_256 as u8)
            .buffers(Buffers::OnlyTx)
            .autopull(true)
            .pull_threshold(32)
            .out_shift_direction(ShiftDirection::Left)
            .build(sm);
        sm.set_pindirs([
            (20, PinDir::Output),
            (21, PinDir::Output),
            (22, PinDir::Output),
        ]);

        // Match pico-dac2: each DMA buffer contains approximately 1 ms of
        // audio, truncated to an integer number of frames.
        let dma_frames = config.rate.hz() as usize / 1_000;
        buffer_a.configure(dma_frames);
        buffer_b.configure(dma_frames);
        fill_dma_buffer(audio, config.format, &mut buffer_a);
        fill_dma_buffer(audio, config.format, &mut buffer_b);

        let transfer = double_buffer::Config::new(channels, buffer_a, tx)
            .start()
            .read_next(buffer_b);
        self.state = Some(State::Running {
            pio,
            sm: sm.start(),
            rx,
            transfer,
            config,
            stop_requested: false,
        });
    }

    pub fn poll(&mut self, audio: &mut AudioEngine) {
        let state = self.state.take().expect("I2S state is always present");
        self.state = Some(match state {
            State::Running {
                pio,
                sm,
                rx,
                transfer,
                config,
                stop_requested,
            } if transfer.is_done() => {
                let (mut completed_buffer, last_transfer) = transfer.wait();
                if stop_requested {
                    State::Draining {
                        pio,
                        sm,
                        rx,
                        transfer: last_transfer,
                        completed_buffer,
                        config,
                    }
                } else {
                    fill_dma_buffer(audio, config.format, &mut completed_buffer);
                    State::Running {
                        pio,
                        sm,
                        rx,
                        transfer: last_transfer.read_next(completed_buffer),
                        config,
                        stop_requested: false,
                    }
                }
            }
            State::Draining {
                mut pio,
                sm,
                rx,
                transfer,
                completed_buffer,
                config: _,
            } if transfer.is_done() => {
                let (channel_a, channel_b, last_buffer, tx) = transfer.wait();
                let mut sm = sm.stop();
                sm.clear_fifos();
                let (sm, installed) = sm.uninit(rx, tx);
                pio.uninstall(installed);
                State::Idle {
                    pio,
                    sm,
                    channels: (channel_a, channel_b),
                    buffer_a: completed_buffer,
                    buffer_b: last_buffer,
                }
            }
            state => state,
        });
    }

    pub fn stop(&mut self) {
        if let Some(State::Running { stop_requested, .. }) = self.state.as_mut() {
            *stop_requested = true;
        }
    }
}

fn program(format: SampleFormat) -> pio::Program<{ pio::RP2040_MAX_PROGRAM_SIZE }> {
    match format {
        SampleFormat::Pcm16 => {
            pio_proc::pio_file!("src/i2s.pio", select_program("i2s_stereo_16bit")).program
        }
        SampleFormat::Pcm24In32 => {
            pio_proc::pio_file!("src/i2s.pio", select_program("i2s_stereo_24bit")).program
        }
        SampleFormat::Pcm32 => {
            pio_proc::pio_file!("src/i2s.pio", select_program("i2s_stereo_32bit")).program
        }
    }
}

const fn cycles_per_frame(format: SampleFormat) -> u32 {
    match format {
        SampleFormat::Pcm16 => 64,
        SampleFormat::Pcm24In32 => 96,
        SampleFormat::Pcm32 => 128,
    }
}

fn fill_dma_buffer(audio: &mut AudioEngine, format: SampleFormat, buffer: &mut DmaBuffer) {
    let mut frames = [StereoFrame::default(); MAX_DMA_FRAMES];
    let frames = &mut frames[..buffer.frames];
    audio.render(frames);
    for (words, frame) in buffer.words_mut().chunks_exact_mut(2).zip(frames) {
        words[0] = frame.left.to_i2s_word(format);
        words[1] = frame.right.to_i2s_word(format);
    }
}
