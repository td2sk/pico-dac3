//! SPDX-License-Identifier: MIT OR Apache-2.0
//!
//! Copyright (c) 2021–2024 The rp-rs Developers
//! Copyright (c) 2021 rp-rs organization
//! Copyright (c) 2025 Raspberry Pi Ltd.
//!
//! pico-dac3 USB Audio Class 2.0 firmware.

#![no_std]
#![no_main]

mod i2s;

use audio_core::{AudioEngine, EngineState, StreamConfig};
use defmt::info;
use defmt_rtt as _;
use embedded_hal::digital::OutputPin;
use hal::{
    Clock,
    dma::{DMAExt, double_buffer},
    pio::{Buffers, PIOBuilder, PIOExt, PinDir, ShiftDirection},
};
#[cfg(target_arch = "riscv32")]
use panic_halt as _;
#[cfg(target_arch = "arm")]
use panic_probe as _;
use static_cell::StaticCell;
use uac2_speaker::{ControlChange, Uac2Event, Uac2Speaker, VendorHid};
use usb_device::{
    bus::UsbBusAllocator,
    device::{UsbDeviceBuilder, UsbRev, UsbVidPid},
};

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

const XTAL_FREQ_HZ: u32 = 12_000_000u32;
const VENDOR_ID: u16 = 0xcafe;
const PRODUCT_ID: u16 = 0xbabe;

static USB_ALLOCATOR: StaticCell<UsbBusAllocator<hal::usb::UsbBus>> = StaticCell::new();
static DMA_BUFFER_A: StaticCell<[u32; i2s::DMA_WORDS]> = StaticCell::new();
static DMA_BUFFER_B: StaticCell<[u32; i2s::DMA_WORDS]> = StaticCell::new();

fn service_status_led<P: OutputPin>(
    pin: &mut P,
    now_us: u32,
    last_toggle_us: &mut u32,
    state: &mut bool,
    audio_state: EngineState,
) {
    let period_us = match audio_state {
        EngineState::Disabled => 1_000_000,
        EngineState::Priming => 500_000,
        EngineState::Running => {
            *state = true;
            let _ = pin.set_high();
            return;
        }
        EngineState::Recovering => 250_000,
    };
    if now_us.wrapping_sub(*last_toggle_us) >= period_us {
        *last_toggle_us = now_us;
        *state = !*state;
        if *state {
            let _ = pin.set_high();
        } else {
            let _ = pin.set_low();
        }
    }
}

#[entry]
fn main() -> ! {
    info!("pico-dac3 start");
    let mut pac = hal::pac::Peripherals::take().unwrap();
    let mut watchdog = hal::Watchdog::new(pac.WATCHDOG);
    let clocks = hal::clocks::init_clocks_and_plls(
        XTAL_FREQ_HZ,
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
    let mut status_led = pins.gpio25.into_push_pull_output();

    #[cfg(rp2040)]
    let timer = hal::Timer::new(pac.TIMER, &mut pac.RESETS, &clocks);
    #[cfg(rp2350)]
    let timer = hal::Timer::new_timer0(pac.TIMER0, &mut pac.RESETS, &clocks);
    let mut led_last_toggle_us = timer.get_counter_low();
    let mut led_state = false;

    let (pio, sm0, _sm1, _sm2, _sm3) = pac.PIO0.split(&mut pac.RESETS);
    let dma = pac.DMA.split(&mut pac.RESETS);
    let buffer_a = DMA_BUFFER_A.init([0; i2s::DMA_WORDS]);
    let buffer_b = DMA_BUFFER_B.init([0; i2s::DMA_WORDS]);
    let mut i2s_resources = Some((pio, sm0, dma.ch0, dma.ch1, buffer_a, buffer_b));
    let system_clock_hz = clocks.system_clock.freq().to_Hz();

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

    let allocator = USB_ALLOCATOR.init(UsbBusAllocator::new(usb_bus));
    let mut uac2 = Uac2Speaker::new(allocator);
    let mut hid = VendorHid::new(allocator);
    let mut usb = UsbDeviceBuilder::new(allocator, UsbVidPid(VENDOR_ID, PRODUCT_ID))
        .composite_with_iads()
        .max_packet_size_0(64)
        .unwrap()
        .usb_rev(UsbRev::Usb200)
        .device_release(0)
        .max_power(100)
        .unwrap()
        .build();
    let mut audio = AudioEngine::new();
    let mut requested_stream: Option<StreamConfig> = None;

    loop {
        // Remain responsive to USB while accumulating the 50% startup level.
        while requested_stream.is_none() || audio.state() != EngineState::Running {
            usb.poll(&mut [&mut uac2, &mut hid]);
            while let Some(event) = uac2.next_event() {
                match event {
                    Uac2Event::StreamStarted { rate, format } => {
                        let config = StreamConfig { rate, format };
                        audio.start(config, rate.hz());
                        requested_stream = Some(config);
                    }
                    Uac2Event::StreamStopped => {
                        audio.stop();
                        requested_stream = None;
                    }
                    Uac2Event::ControlChanged(change) => match change {
                        ControlChange::SampleRate(_) => {
                            audio.stop();
                            requested_stream = None;
                        }
                        ControlChange::Mute { channel, value } => audio.set_mute(channel, value),
                        ControlChange::Volume { channel, value } => {
                            audio.set_volume(channel, value)
                        }
                    },
                }
            }
            if let Some(packet) = uac2.take_packet() {
                let _ = audio.push_usb_packet(packet);
            }
            let _ = hid.take_output_report();
            if uac2.feedback_requested() {
                uac2.set_feedback(audio.feedback());
            }
            service_status_led(
                &mut status_led,
                timer.get_counter_low(),
                &mut led_last_toggle_us,
                &mut led_state,
                audio.state(),
            );
        }

        let config = requested_stream.take().unwrap();
        let (mut pio, sm0, ch0, ch1, buffer_a, buffer_b) = i2s_resources.take().unwrap();

        let pio_program = i2s::program(config.format);
        let cycles_per_frame = i2s::cycles_per_frame(config.format);
        let divider_256 = ((system_clock_hz as u64 * 256)
            / (config.rate.hz() as u64 * cycles_per_frame as u64)) as u32;
        let actual_rate_hz = ((system_clock_hz as u64 * 256)
            / (divider_256 as u64 * cycles_per_frame as u64)) as u32;
        audio.set_actual_output_rate(actual_rate_hz);
        let installed = pio.install(&pio_program).unwrap();
        let (mut sm, rx, tx) = PIOBuilder::from_installed_program(installed)
            .out_pins(22, 1)
            .side_set_pin_base(20)
            .clock_divisor_fixed_point((divider_256 / 256) as u16, divider_256 as u8)
            .buffers(Buffers::OnlyTx)
            .autopull(true)
            .pull_threshold(32)
            .out_shift_direction(ShiftDirection::Left)
            .build(sm0);
        sm.set_pindirs([
            (20, PinDir::Output),
            (21, PinDir::Output),
            (22, PinDir::Output),
        ]);

        i2s::fill_dma_buffer(&mut audio, config.format, buffer_a);
        i2s::fill_dma_buffer(&mut audio, config.format, buffer_b);
        let sm = sm.start();
        let transfer = double_buffer::Config::new((ch0, ch1), buffer_a, tx).start();
        let mut transfer = transfer.read_next(buffer_b);
        let mut restart_stream = None;

        loop {
            usb.poll(&mut [&mut uac2, &mut hid]);
            let mut stop = false;
            while let Some(event) = uac2.next_event() {
                match event {
                    Uac2Event::StreamStarted { rate, format } => {
                        restart_stream = Some(StreamConfig { rate, format });
                        stop = true;
                    }
                    Uac2Event::StreamStopped => {
                        audio.stop();
                        stop = true;
                    }
                    Uac2Event::ControlChanged(change) => match change {
                        ControlChange::SampleRate(_) => {
                            audio.stop();
                            stop = true;
                        }
                        ControlChange::Mute { channel, value } => audio.set_mute(channel, value),
                        ControlChange::Volume { channel, value } => {
                            audio.set_volume(channel, value)
                        }
                    },
                }
            }
            if let Some(packet) = uac2.take_packet() {
                let _ = audio.push_usb_packet(packet);
            }
            let _ = hid.take_output_report();
            if uac2.feedback_requested() {
                uac2.set_feedback(audio.feedback());
            }
            service_status_led(
                &mut status_led,
                timer.get_counter_low(),
                &mut led_last_toggle_us,
                &mut led_state,
                audio.state(),
            );

            if stop {
                break;
            }
            if transfer.is_done() {
                let (free_buffer, next) = transfer.wait();
                i2s::fill_dma_buffer(&mut audio, config.format, free_buffer);
                transfer = next.read_next(free_buffer);
            }
        }

        // Drain the two already-scheduled blocks before reclaiming DMA/PIO.
        let (buffer_a, transfer) = transfer.wait();
        let (ch0, ch1, buffer_b, tx) = transfer.wait();
        let (sm0, installed) = sm.stop().uninit(rx, tx);
        pio.uninstall(installed);
        i2s_resources = Some((pio, sm0, ch0, ch1, buffer_a, buffer_b));

        if let Some(config) = restart_stream {
            audio.start(config, config.rate.hz());
            requested_stream = Some(config);
        }
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
