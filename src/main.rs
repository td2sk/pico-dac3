//! SPDX-License-Identifier: MIT
//!
//! Copyright (c) 2026 td2sk
//!
//! pico-dac3 USB Audio Class 2.0 firmware.

#![no_std]
#![no_main]

mod clocks;
mod i2s;
mod status_led;
mod stream;
mod usb;

use audio_core::{AudioEngine, EngineState};
use defmt::info;
use defmt_rtt as _;
use hal::{dma::DMAExt, pio::PIOExt};
#[cfg(target_arch = "riscv32")]
use panic_halt as _;
#[cfg(target_arch = "arm")]
use panic_probe as _;
use static_cell::StaticCell;
use status_led::{LedState, StatusLed};
use stream::StreamController;
use usb::UsbRuntime;

// Alias for our HAL crate
use hal::entry;

#[cfg(rp2350)]
use rp235x_hal as hal;

#[cfg(rp2040)]
use rp2040_hal as hal;

// use bsp::entry;
// use bsp::hal;
// use rp_pico as bsp;

/// The linker will place this boot block at the start of our program image. We
/// need this to help the ROM bootloader get our code up and running.
/// Note: This boot block is not necessary when using a rp-hal based BSP
/// as the BSPs already perform this step.
#[unsafe(link_section = ".boot2")]
#[used]
#[cfg(rp2040)]
pub static BOOT2: [u8; 256] = rp2040_boot2::BOOT_LOADER_W25Q080;

/// Tell the Boot ROM about our application
#[unsafe(link_section = ".start_block")]
#[used]
#[cfg(rp2350)]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

static DMA_BUFFER_A: StaticCell<[u32; i2s::MAX_DMA_WORDS]> = StaticCell::new();
static DMA_BUFFER_B: StaticCell<[u32; i2s::MAX_DMA_WORDS]> = StaticCell::new();

const fn status_led_state(audio_state: EngineState) -> LedState {
    match audio_state {
        EngineState::Disabled => LedState::Blink {
            toggle_interval_us: 1_000_000,
        },
        EngineState::Priming => LedState::Blink {
            toggle_interval_us: 500_000,
        },
        EngineState::Running => LedState::On,
        EngineState::Recovering => LedState::Blink {
            toggle_interval_us: 250_000,
        },
    }
}

#[entry]
fn main() -> ! {
    info!("pico-dac3 start");
    let mut pac = hal::pac::Peripherals::take().unwrap();
    let mut watchdog = hal::Watchdog::new(pac.WATCHDOG);
    let clocks = clocks::init(
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .unwrap();

    // GPIO must be released from reset before USB is enabled (also required by
    // the RP2040-E5 workaround when that HAL feature is selected).
    let sio = hal::Sio::new(pac.SIO);
    let pins = hal::gpio::Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );
    let _i2s_bclk: hal::gpio::Pin<_, hal::gpio::FunctionPio0, _> = pins.gpio20.into_function();
    let _i2s_lrclk: hal::gpio::Pin<_, hal::gpio::FunctionPio0, _> = pins.gpio21.into_function();
    let _i2s_data: hal::gpio::Pin<_, hal::gpio::FunctionPio0, _> = pins.gpio22.into_function();
    let status_led_pin = pins.gpio25.into_push_pull_output();

    #[cfg(rp2040)]
    let timer = hal::Timer::new(pac.TIMER, &mut pac.RESETS, &clocks);
    #[cfg(rp2350)]
    let timer = hal::Timer::new_timer0(pac.TIMER0, &mut pac.RESETS, &clocks);
    let mut status_led = StatusLed::new(status_led_pin, timer.get_counter_low());

    let (pio, sm0, _sm1, _sm2, _sm3) = pac.PIO0.split(&mut pac.RESETS);
    let dma = pac.DMA.split(&mut pac.RESETS);
    let buffer_a = DMA_BUFFER_A.init([0; i2s::MAX_DMA_WORDS]);
    let buffer_b = DMA_BUFFER_B.init([0; i2s::MAX_DMA_WORDS]);
    // `clocks.rs` intentionally bypasses rp-hal's VCO range check to reproduce
    // pico-dac2, so the HAL's typed bootstrap token does not carry this rate.
    let system_clock_hz = clocks::SYSTEM_CLOCK_HZ;
    let mut i2s = i2s::I2s::new(
        pio,
        sm0,
        (dma.ch0, dma.ch1),
        buffer_a,
        buffer_b,
        system_clock_hz,
    );

    #[cfg(rp2040)]
    let usb_bus = hal::usb::UsbBus::new(
        pac.USBCTRL_REGS,
        pac.USBCTRL_DPRAM,
        clocks.usb_clock,
        true,
        &mut pac.RESETS,
    );
    #[cfg(rp2350)]
    let usb_bus = hal::usb::UsbBus::new(
        pac.USB,
        pac.USB_DPRAM,
        clocks.usb_clock,
        true,
        &mut pac.RESETS,
    );

    let mut usb = UsbRuntime::new(usb_bus);
    let mut audio = AudioEngine::new();
    let mut stream = StreamController::new();

    loop {
        usb.poll();
        while let Some(event) = usb.next_audio_event() {
            stream.handle_event(event, usb.current_format(), &mut audio);
        }

        if let Some(packet) = usb.take_audio_packet() {
            let _ = audio.push_usb_packet(packet);
        }
        let _ = usb.take_hid_output_report();

        stream.reconcile(&mut audio, &mut i2s);
        i2s.poll(&mut audio);

        if usb.feedback_requested() {
            usb.set_feedback(audio.feedback());
        }
        status_led.update(timer.get_counter_low(), status_led_state(audio.state()));
    }
}

/// Program metadata for `picotool info`
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 5] = [
    hal::binary_info::rp_cargo_bin_name!(),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_description!(c"pico-dac3 UAC2 USB DAC"),
    hal::binary_info::rp_cargo_homepage_url!(),
    hal::binary_info::rp_program_build_attribute!(),
];

// End of file
