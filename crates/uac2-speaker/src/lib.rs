#![no_std]
#![deny(unsafe_code)]

mod hid;
mod hid_descriptors;
mod uac2;
mod uac2_descriptors;

pub use hid::VendorHid;
pub use hid_descriptors::HID_REPORT_DESCRIPTOR;
pub use uac2::{ControlChange, Uac2Event, Uac2Speaker};
pub use uac2_descriptors::AUDIO_MAX_PACKET_SIZE;
