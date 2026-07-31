use audio_core::{AudioEngine, SampleFormat, StereoFrame};

pub const DMA_FRAMES: usize = 96;
pub const DMA_WORDS: usize = DMA_FRAMES * 2;

pub fn program(format: SampleFormat) -> pio::Program<{ pio::RP2040_MAX_PROGRAM_SIZE }> {
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

pub const fn cycles_per_frame(format: SampleFormat) -> u32 {
    match format {
        SampleFormat::Pcm16 => 64,
        SampleFormat::Pcm24In32 => 96,
        SampleFormat::Pcm32 => 128,
    }
}

pub fn fill_dma_buffer(
    audio: &mut AudioEngine,
    format: SampleFormat,
    buffer: &mut [u32; DMA_WORDS],
) {
    let mut frames = [StereoFrame::default(); DMA_FRAMES];
    audio.render(&mut frames);
    for (words, frame) in buffer.chunks_exact_mut(2).zip(frames) {
        words[0] = frame.left.to_i2s_word(format);
        words[1] = frame.right.to_i2s_word(format);
    }
}
