#![no_std]
#![deny(unsafe_code)]

mod descriptors;

use audio_core::{Channel, FeedbackQ16, SampleFormat, SampleRate, Volume};
use usb_device::{
    Result as UsbResult, UsbDirection, UsbError,
    bus::{InterfaceNumber, UsbBus, UsbBusAllocator},
    class_prelude::{ControlIn, ControlOut, DescriptorWriter, EndpointIn, EndpointOut, UsbClass},
    control::{Recipient, RequestType},
    endpoint::{
        EndpointAddress, EndpointType, IsochronousSynchronizationType, IsochronousUsageType,
    },
};

pub use descriptors::{AUDIO_MAX_PACKET_SIZE, HID_REPORT_DESCRIPTOR};

const FEATURE_UNIT: u8 = 2;
const CLOCK_SOURCE: u8 = 4;
const CUR: u8 = 1;
const RANGE: u8 = 2;
const MUTE: u8 = 1;
const VOLUME: u8 = 2;
const SAMPLING_FREQUENCY: u8 = 1;
const CLOCK_VALIDITY: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlChange {
    SampleRate(SampleRate),
    Mute { channel: Channel, value: bool },
    Volume { channel: Channel, value: Volume },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Uac2Event {
    StreamStarted {
        rate: SampleRate,
        format: SampleFormat,
    },
    StreamStopped,
    ControlChanged(ControlChange),
}

pub struct Uac2Speaker<'a, B: UsbBus> {
    control_interface: InterfaceNumber,
    streaming_interface: InterfaceNumber,
    audio_out: EndpointOut<'a, B>,
    feedback_in: EndpointIn<'a, B>,
    alt: u8,
    rate: SampleRate,
    mute: [bool; 3],
    volume: [Volume; 3],
    event: Option<Uac2Event>,
    packet: [u8; AUDIO_MAX_PACKET_SIZE as usize],
    packet_len: usize,
    feedback: FeedbackQ16,
    feedback_pending: bool,
}

impl<'a, B: UsbBus> Uac2Speaker<'a, B> {
    pub fn new(alloc: &'a UsbBusAllocator<B>) -> Self {
        let control_interface = alloc.interface();
        let streaming_interface = alloc.interface();
        let audio_out = alloc
            .alloc(
                Some(EndpointAddress::from_parts(1, UsbDirection::Out)),
                EndpointType::Isochronous {
                    synchronization: IsochronousSynchronizationType::Asynchronous,
                    usage: IsochronousUsageType::Data,
                },
                AUDIO_MAX_PACKET_SIZE,
                1,
            )
            .expect("audio OUT endpoint allocation failed");
        let feedback_in = alloc
            .alloc(
                Some(EndpointAddress::from_parts(1, UsbDirection::In)),
                EndpointType::Isochronous {
                    synchronization: IsochronousSynchronizationType::NoSynchronization,
                    usage: IsochronousUsageType::Feedback,
                },
                4,
                1,
            )
            .expect("feedback endpoint allocation failed");
        Self {
            control_interface,
            streaming_interface,
            audio_out,
            feedback_in,
            alt: 0,
            rate: SampleRate::Hz48000,
            mute: [false; 3],
            volume: [Volume::MAX; 3],
            event: None,
            packet: [0; AUDIO_MAX_PACKET_SIZE as usize],
            packet_len: 0,
            feedback: FeedbackQ16::from_rate_hz(48_000),
            feedback_pending: false,
        }
    }

    pub fn next_event(&mut self) -> Option<Uac2Event> {
        self.event.take()
    }

    pub fn take_packet(&mut self) -> Option<&[u8]> {
        if self.packet_len == 0 {
            return None;
        }
        let len = core::mem::take(&mut self.packet_len);
        Some(&self.packet[..len])
    }

    pub fn set_feedback(&mut self, value: FeedbackQ16) {
        self.feedback = value;
    }

    pub const fn feedback_requested(&self) -> bool {
        self.feedback_pending
    }

    pub const fn current_format(&self) -> Option<SampleFormat> {
        match self.alt {
            1 => Some(SampleFormat::Pcm16),
            2 => Some(SampleFormat::Pcm24In32),
            3 => Some(SampleFormat::Pcm32),
            _ => None,
        }
    }

    fn channel(raw: u8) -> Option<(usize, Channel)> {
        match raw {
            0 => Some((0, Channel::Master)),
            1 => Some((1, Channel::Left)),
            2 => Some((2, Channel::Right)),
            _ => None,
        }
    }

    fn owns(&self, req: &usb_device::control::Request) -> bool {
        req.request_type == RequestType::Class
            && req.recipient == Recipient::Interface
            && req.index as u8 == u8::from(self.control_interface)
    }
}

impl<B: UsbBus> UsbClass<B> for Uac2Speaker<'_, B> {
    fn get_configuration_descriptors(&self, writer: &mut DescriptorWriter) -> UsbResult<()> {
        descriptors::write_uac2(
            writer,
            self.control_interface,
            self.streaming_interface,
            &self.audio_out,
            &self.feedback_in,
        )
    }

    fn reset(&mut self) {
        self.alt = 0;
        self.packet_len = 0;
        self.event = Some(Uac2Event::StreamStopped);
    }

    fn get_alt_setting(&mut self, interface: InterfaceNumber) -> Option<u8> {
        (u8::from(interface) == u8::from(self.streaming_interface)).then_some(self.alt)
    }

    fn set_alt_setting(&mut self, interface: InterfaceNumber, alternative: u8) -> bool {
        if u8::from(interface) != u8::from(self.streaming_interface) || alternative > 3 {
            return false;
        }
        self.alt = alternative;
        self.packet_len = 0;
        self.event = if alternative == 0 {
            self.feedback_pending = false;
            Some(Uac2Event::StreamStopped)
        } else {
            self.feedback_pending = true;
            Some(Uac2Event::StreamStarted {
                rate: self.rate,
                format: self.current_format().unwrap(),
            })
        };
        true
    }

    fn endpoint_out(&mut self, address: EndpointAddress) {
        if address == self.audio_out.address() {
            match self.audio_out.read(&mut self.packet) {
                Ok(len) => self.packet_len = len,
                Err(UsbError::WouldBlock) => {}
                Err(_) => self.packet_len = 0,
            }
        }
    }

    fn poll(&mut self) {
        if self.alt != 0
            && self.feedback_pending
            && self
                .feedback_in
                .write(&self.feedback.to_usb_bytes())
                .is_ok()
        {
            self.feedback_pending = false;
        }
    }

    fn endpoint_in_complete(&mut self, address: EndpointAddress) {
        if address == self.feedback_in.address() && self.alt != 0 {
            self.feedback_pending = true;
        }
    }

    fn control_in(&mut self, xfer: ControlIn<B>) {
        let req = *xfer.request();
        if !self.owns(&req) {
            return;
        }
        let entity = (req.index >> 8) as u8;
        let selector = (req.value >> 8) as u8;
        let channel = req.value as u8;
        let result = if entity == CLOCK_SOURCE {
            match (selector, req.request) {
                (SAMPLING_FREQUENCY, CUR) => xfer.accept_with(&self.rate.hz().to_le_bytes()),
                (SAMPLING_FREQUENCY, RANGE) => xfer.accept_with(&descriptors::sample_rate_range()),
                (CLOCK_VALIDITY, CUR) => xfer.accept_with(&[1]),
                _ => xfer.reject(),
            }
        } else if entity == FEATURE_UNIT {
            let Some((index, _)) = Self::channel(channel) else {
                let _ = xfer.reject();
                return;
            };
            match (selector, req.request) {
                (MUTE, CUR) => xfer.accept_with(&[self.mute[index] as u8]),
                (VOLUME, CUR) => xfer.accept_with(&self.volume[index].db_256().to_le_bytes()),
                (VOLUME, RANGE) => xfer.accept_with(&descriptors::volume_range()),
                _ => xfer.reject(),
            }
        } else {
            xfer.reject()
        };
        let _ = result;
    }

    fn control_out(&mut self, xfer: ControlOut<B>) {
        let req = *xfer.request();
        if !self.owns(&req) {
            return;
        }
        let entity = (req.index >> 8) as u8;
        let selector = (req.value >> 8) as u8;
        let raw_channel = req.value as u8;

        if entity == CLOCK_SOURCE
            && selector == SAMPLING_FREQUENCY
            && req.request == CUR
            && xfer.data().len() == 4
        {
            let hz = u32::from_le_bytes(xfer.data().try_into().unwrap());
            if let Some(rate) = SampleRate::from_hz(hz) {
                self.rate = rate;
                self.event = Some(Uac2Event::ControlChanged(ControlChange::SampleRate(rate)));
                let _ = xfer.accept();
            } else {
                let _ = xfer.reject();
            }
            return;
        }

        if entity == FEATURE_UNIT && req.request == CUR {
            let Some((index, channel)) = Self::channel(raw_channel) else {
                let _ = xfer.reject();
                return;
            };
            match (selector, xfer.data()) {
                (MUTE, [value]) if *value <= 1 => {
                    self.mute[index] = *value != 0;
                    self.event = Some(Uac2Event::ControlChanged(ControlChange::Mute {
                        channel,
                        value: *value != 0,
                    }));
                    let _ = xfer.accept();
                }
                (VOLUME, [lo, hi]) => {
                    if let Some(value) = Volume::from_db_256(i16::from_le_bytes([*lo, *hi])) {
                        self.volume[index] = value;
                        self.event = Some(Uac2Event::ControlChanged(ControlChange::Volume {
                            channel,
                            value,
                        }));
                        let _ = xfer.accept();
                    } else {
                        let _ = xfer.reject();
                    }
                }
                _ => {
                    let _ = xfer.reject();
                }
            }
            return;
        }
        let _ = xfer.reject();
    }
}

pub struct VendorHid<'a, B: UsbBus> {
    interface: InterfaceNumber,
    input: EndpointIn<'a, B>,
    output: EndpointOut<'a, B>,
    output_report: [u8; 16],
    output_ready: bool,
}

impl<'a, B: UsbBus> VendorHid<'a, B> {
    pub fn new(alloc: &'a UsbBusAllocator<B>) -> Self {
        let interface = alloc.interface();
        let input = alloc
            .alloc(
                Some(EndpointAddress::from_parts(2, UsbDirection::In)),
                EndpointType::Interrupt,
                16,
                200,
            )
            .expect("HID IN endpoint allocation failed");
        let output = alloc
            .alloc(
                Some(EndpointAddress::from_parts(2, UsbDirection::Out)),
                EndpointType::Interrupt,
                16,
                200,
            )
            .expect("HID OUT endpoint allocation failed");
        Self {
            interface,
            input,
            output,
            output_report: [0; 16],
            output_ready: false,
        }
    }

    pub fn take_output_report(&mut self) -> Option<[u8; 16]> {
        if core::mem::take(&mut self.output_ready) {
            Some(self.output_report)
        } else {
            None
        }
    }

    pub fn write_input_report(&self, report: &[u8; 16]) -> UsbResult<usize> {
        self.input.write(report)
    }
}

impl<B: UsbBus> UsbClass<B> for VendorHid<'_, B> {
    fn get_configuration_descriptors(&self, writer: &mut DescriptorWriter) -> UsbResult<()> {
        writer.interface(self.interface, 0x03, 0x00, 0x00)?;
        writer.write(
            0x21,
            &[
                0x11,
                0x01, // HID 1.11
                0x00, // country
                0x01, // descriptor count
                0x22, // report descriptor
                HID_REPORT_DESCRIPTOR.len() as u8,
                0x00,
            ],
        )?;
        writer.endpoint(&self.input)?;
        writer.endpoint(&self.output)
    }

    fn control_in(&mut self, xfer: ControlIn<B>) {
        let req = *xfer.request();
        let (descriptor_type, _) = req.descriptor_type_index();
        if req.request_type == RequestType::Standard
            && req.recipient == Recipient::Interface
            && req.index as u8 == u8::from(self.interface)
            && req.request == usb_device::control::Request::GET_DESCRIPTOR
            && descriptor_type == 0x22
        {
            let _ = xfer.accept_with_static(HID_REPORT_DESCRIPTOR);
        }
    }

    fn control_out(&mut self, xfer: ControlOut<B>) {
        let req = *xfer.request();
        if req.request_type == RequestType::Class
            && req.recipient == Recipient::Interface
            && req.index as u8 == u8::from(self.interface)
            && req.request == 0x0a
        {
            let _ = xfer.accept();
        }
    }

    fn endpoint_out(&mut self, address: EndpointAddress) {
        if address == self.output.address()
            && let Ok(16) = self.output.read(&mut self.output_report)
        {
            self.output_ready = true;
        }
    }

    fn endpoint_in_complete(&mut self, address: EndpointAddress) {
        if address == self.input.address() {
            let _ = self.input.write(&[0; 16]);
        }
    }

    fn poll(&mut self) {
        let _ = self.input.write(&[0; 16]);
    }
}
