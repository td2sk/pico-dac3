use usb_device::{
    Result as UsbResult, UsbDirection,
    bus::{InterfaceNumber, UsbBus, UsbBusAllocator},
    class_prelude::{ControlIn, ControlOut, DescriptorWriter, EndpointIn, EndpointOut, UsbClass},
    control::{Recipient, RequestType},
    endpoint::{EndpointAddress, EndpointType},
};

use crate::hid_descriptors::{self as descriptors, HID_REPORT_DESCRIPTOR};

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
        descriptors::write_hid(writer, self.interface, &self.input, &self.output)
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
