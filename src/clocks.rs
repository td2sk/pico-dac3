use crate::hal;
use hal::{
    clocks::{Clock, ClockSource, ClocksManager, InitError},
    fugit::RateExtU32,
    pll::{PLLConfig, common_configs::PLL_USB_48MHZ, setup_pll_blocking},
};

pub const XOSC_HZ: u32 = 12_000_000;
pub const SYSTEM_CLOCK_HZ: u32 = 92_160_000;

/// Reproduce pico-dac2's 92.16 MHz system clock without exceeding the PLL VCO
/// range enforced by rp-hal. The C firmware generates 92.16 MHz directly from
/// a 2304 MHz VCO; here a 1152 MHz VCO produces 115.2 MHz, followed by the
/// clock block's exact 1.25 divider.
pub fn init(
    xosc_dev: hal::pac::XOSC,
    clocks_dev: hal::pac::CLOCKS,
    pll_sys_dev: hal::pac::PLL_SYS,
    pll_usb_dev: hal::pac::PLL_USB,
    resets: &mut hal::pac::RESETS,
    watchdog: &mut hal::Watchdog,
) -> Result<ClocksManager, InitError> {
    let xosc =
        hal::xosc::setup_xosc_blocking(xosc_dev, XOSC_HZ.Hz()).map_err(InitError::XoscErr)?;

    #[cfg(rp2040)]
    watchdog.enable_tick_generation((XOSC_HZ / 1_000_000) as u8);
    #[cfg(rp2350)]
    watchdog.enable_tick_generation((XOSC_HZ / 1_000_000) as u16);

    let mut clocks = ClocksManager::new(clocks_dev);
    let pll_sys = setup_pll_blocking(
        pll_sys_dev,
        xosc.operating_frequency(),
        PLLConfig {
            vco_freq: 1_152_000_000_u32.Hz(),
            refdiv: 1,
            post_div1: 5,
            post_div2: 2,
        },
        &mut clocks,
        resets,
    )
    .map_err(InitError::PllError)?;
    let pll_usb = setup_pll_blocking(
        pll_usb_dev,
        xosc.operating_frequency(),
        PLL_USB_48MHZ,
        &mut clocks,
        resets,
    )
    .map_err(InitError::PllError)?;

    clocks
        .reference_clock
        .configure_clock(&xosc, xosc.get_freq())
        .map_err(InitError::ClockError)?;
    clocks
        .system_clock
        .configure_clock(&pll_sys, SYSTEM_CLOCK_HZ.Hz())
        .map_err(InitError::ClockError)?;
    clocks
        .usb_clock
        .configure_clock(&pll_usb, pll_usb.get_freq())
        .map_err(InitError::ClockError)?;
    clocks
        .adc_clock
        .configure_clock(&pll_usb, pll_usb.get_freq())
        .map_err(InitError::ClockError)?;
    #[cfg(rp2040)]
    clocks
        .rtc_clock
        .configure_clock(&pll_usb, 46_875_u32.Hz())
        .map_err(InitError::ClockError)?;
    clocks
        .peripheral_clock
        .configure_clock(&clocks.system_clock, clocks.system_clock.freq())
        .map_err(InitError::ClockError)?;

    Ok(clocks)
}
