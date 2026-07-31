use usb_device::{
    Result,
    bus::{InterfaceNumber, UsbBus},
    class_prelude::{DescriptorWriter, EndpointIn, EndpointOut},
};

pub const HID_REPORT_DESCRIPTOR: &[u8] = &[
    0x06, 0x00, 0xff, // Usage Page: Vendor Defined 0xff00
    0x09, 0x01, // Usage: 1
    0xa1, 0x01, // Collection: Application
    0x09, 0x02, //   Usage: 2 (input)
    0x15, 0x00, //   Logical Minimum: 0
    0x26, 0xff, 0x00, //   Logical Maximum: 255
    0x75, 0x08, //   Report Size: 8 bits
    0x95, 0x10, //   Report Count: 16
    0x81, 0x02, //   Input: Data, Variable, Absolute
    0x09, 0x03, //   Usage: 3 (output)
    0x15, 0x00, //   Logical Minimum: 0
    0x26, 0xff, 0x00, //   Logical Maximum: 255
    0x75, 0x08, //   Report Size: 8 bits
    0x95, 0x10, //   Report Count: 16
    0x91, 0x02, //   Output: Data, Variable, Absolute
    0xc0, // End Collection
];

pub fn write_hid<B: UsbBus>(
    writer: &mut DescriptorWriter,
    interface: InterfaceNumber,
    input: &EndpointIn<'_, B>,
    output: &EndpointOut<'_, B>,
) -> Result<()> {
    writer.interface(interface, 0x03, 0x00, 0x00)?;
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
    writer.endpoint(input)?;
    writer.endpoint(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_is_vendor_defined_16_byte_in_out() {
        assert_eq!(HID_REPORT_DESCRIPTOR.len(), 34);
        assert_eq!(
            HID_REPORT_DESCRIPTOR
                .windows(2)
                .filter(|pair| *pair == [0x95, 0x10])
                .count(),
            2
        );
    }
}
