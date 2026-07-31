use embedded_hal::digital::OutputPin;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LedState {
    On,
    Blink { toggle_interval_us: u32 },
}

pub struct StatusLed<P> {
    pin: P,
    last_toggle_us: u32,
    is_on: bool,
}

impl<P: OutputPin> StatusLed<P> {
    pub fn new(pin: P, now_us: u32) -> Self {
        Self {
            pin,
            last_toggle_us: now_us,
            is_on: false,
        }
    }

    pub fn update(&mut self, now_us: u32, state: LedState) {
        match state {
            LedState::On => self.set(true),
            LedState::Blink { toggle_interval_us }
                if now_us.wrapping_sub(self.last_toggle_us) >= toggle_interval_us =>
            {
                self.last_toggle_us = self.last_toggle_us.wrapping_add(toggle_interval_us);
                self.set(!self.is_on);
            }
            LedState::Blink { .. } => {}
        }
    }

    fn set(&mut self, on: bool) {
        self.is_on = on;
        if on {
            let _ = self.pin.set_high();
        } else {
            let _ = self.pin.set_low();
        }
    }
}
