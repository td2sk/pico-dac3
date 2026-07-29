//! SPDX-License-Identifier: MIT OR Apache-2.0
//!
//! Copyright (c) 2021–2024 The rp-rs Developers
//! Copyright (c) 2021 rp-rs organization
//! Copyright (c) 2025 Raspberry Pi Ltd.
//!
//! pico-dac3 USB Audio Class 2.0 firmware.

#![no_std]
#![no_main]

use audio_core::{AudioEngine, StreamConfig};
use defmt::info;
use defmt_rtt as _;
#[cfg(target_arch = "riscv32")]
use panic_halt as _;
#[cfg(target_arch = "arm")]
use panic_probe as _;
use static_cell::StaticCell;
use uac2_speaker::{ControlChange, Uac2Event, Uac2Speaker};
use usb_device::{
    bus::UsbBusAllocator,
    device::{UsbDeviceBuilder, UsbVidPid},
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
    let _pins = hal::gpio::Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
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

    let allocator = USB_ALLOCATOR.init(UsbBusAllocator::new(usb_bus));
    let mut uac2 = Uac2Speaker::new(allocator);
    let mut usb = UsbDeviceBuilder::new(allocator, UsbVidPid(VENDOR_ID, PRODUCT_ID))
        .composite_with_iads()
        .max_packet_size_0(64)
        .unwrap()
        .max_power(100)
        .unwrap()
        .build();
    let mut audio = AudioEngine::new();

    loop {
        usb.poll(&mut [&mut uac2]);

        while let Some(event) = uac2.next_event() {
            match event {
                Uac2Event::StreamStarted { rate, format } => {
                    // The I2S driver replaces this nominal rate with the exact
                    // PIO divider result when the stream is configured.
                    audio.start(StreamConfig { rate, format }, rate.hz());
                }
                Uac2Event::StreamStopped => audio.stop(),
                Uac2Event::ControlChanged(change) => match change {
                    ControlChange::SampleRate(_) => audio.stop(),
                    ControlChange::Mute { channel, value } => audio.set_mute(channel, value),
                    ControlChange::Volume { channel, value } => audio.set_volume(channel, value),
                },
            }
        }

        if let Some(packet) = uac2.take_packet() {
            let _ = audio.push_usb_packet(packet);
        }
        uac2.set_feedback(audio.feedback());
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
