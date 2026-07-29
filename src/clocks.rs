use crate::hal;
use hal::{
    clocks::{Clock, ClockSource, ClocksManager, InitError},
    fugit::RateExtU32,
    pll::{PLLConfig, common_configs::PLL_USB_48MHZ, setup_pll_blocking},
};

pub const XOSC_HZ: u32 = 12_000_000;
pub const SYSTEM_CLOCK_HZ: u32 = 92_160_000;

/// Reprogram PLL_SYS exactly as pico-dac2's
/// `set_sys_clock_pll(12 * MHz * 192, 5, 5)`.
///
/// rp-hal deliberately rejects a 2304 MHz VCO, so only this explicitly
/// requested out-of-range configuration crosses an unsafe register boundary.
unsafe fn configure_legacy_pll_sys() {
    // SAFETY: PLL_SYS is owned by `init`; CLK_SYS is still running from its
    // reset source while this function powers down and reconfigures PLL_SYS.
    let pll = unsafe { &*hal::pac::PLL_SYS::ptr() };

    pll.pwr().reset();
    pll.fbdiv_int().reset();
    pll.cs().write(|w| unsafe { w.refdiv().bits(1) });
    pll.fbdiv_int()
        .write(|w| unsafe { w.fbdiv_int().bits(192) });
    pll.pwr().modify(|_, w| {
        w.pd().clear_bit();
        w.vcopd().clear_bit()
    });
    while pll.cs().read().lock().bit_is_clear() {}
    pll.prim().write(|w| unsafe {
        w.postdiv1().bits(5);
        w.postdiv2().bits(5)
    });
    pll.pwr().modify(|_, w| w.postdivpd().clear_bit());
}

/// Reproduce pico-dac2's exact 2304 MHz / 5 / 5 PLL_SYS configuration.
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
    // Create a typed PLL source through HAL first. Its configuration is only a
    // bootstrap accepted by HAL; it is replaced below before CLK_SYS selects it.
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

    // SAFETY: CLK_SYS has not selected PLL_SYS yet.
    unsafe { configure_legacy_pll_sys() };

    clocks
        .reference_clock
        .configure_clock(&xosc, xosc.get_freq())
        .map_err(InitError::ClockError)?;
    clocks
        .system_clock
        // Request the bootstrap token's full rate so HAL programs a divider of
        // one. The physical PLL was replaced above and outputs 92.16 MHz.
        .configure_clock(&pll_sys, pll_sys.get_freq())
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
        // Pico SDK's set_sys_clock_pll() keeps clk_peri on PLL_USB unless
        // PICO_CLOCK_ADJUST_PERI_CLOCK_WITH_SYS_CLOCK is enabled.
        .configure_clock(&pll_usb, pll_usb.get_freq())
        .map_err(InitError::ClockError)?;

    Ok(clocks)
}
