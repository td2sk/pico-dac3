use audio_core::{FeedbackQ16, SampleFormat};
use static_cell::StaticCell;
use uac2_speaker::{Uac2Event, Uac2Speaker, VendorHid};
use usb_device::{
    bus::UsbBusAllocator,
    device::{UsbDevice, UsbDeviceBuilder, UsbRev, UsbVidPid},
};

use crate::hal;

const VENDOR_ID: u16 = 0xcafe;
const PRODUCT_ID: u16 = 0xbabe;

type Bus = hal::usb::UsbBus;

static USB_ALLOCATOR: StaticCell<UsbBusAllocator<Bus>> = StaticCell::new();

pub struct UsbRuntime {
    device: UsbDevice<'static, Bus>,
    uac2: Uac2Speaker<'static, Bus>,
    hid: VendorHid<'static, Bus>,
}

impl UsbRuntime {
    pub fn new(bus: Bus) -> Self {
        let allocator = USB_ALLOCATOR.init(UsbBusAllocator::new(bus));
        let uac2 = Uac2Speaker::new(allocator);
        let hid = VendorHid::new(allocator);
        let device = UsbDeviceBuilder::new(allocator, UsbVidPid(VENDOR_ID, PRODUCT_ID))
            .composite_with_iads()
            .max_packet_size_0(64)
            .expect("EP0 packet size is valid")
            .usb_rev(UsbRev::Usb200)
            .device_release(0)
            .max_power(100)
            .expect("USB power consumption is valid")
            .build();
        Self { device, uac2, hid }
    }

    pub fn poll(&mut self) {
        self.device.poll(&mut [&mut self.uac2, &mut self.hid]);
    }

    pub fn next_audio_event(&mut self) -> Option<Uac2Event> {
        self.uac2.next_event()
    }

    pub const fn current_format(&self) -> Option<SampleFormat> {
        self.uac2.current_format()
    }

    pub fn take_audio_packet(&mut self) -> Option<&[u8]> {
        self.uac2.take_packet()
    }

    pub fn take_hid_output_report(&mut self) -> Option<[u8; 16]> {
        self.hid.take_output_report()
    }

    pub const fn feedback_requested(&self) -> bool {
        self.uac2.feedback_requested()
    }

    pub fn set_feedback(&mut self, feedback: FeedbackQ16) {
        self.uac2.set_feedback(feedback);
    }
}
