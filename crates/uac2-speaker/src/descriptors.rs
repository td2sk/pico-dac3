use audio_core::Volume;
use usb_device::{
    Result,
    bus::{InterfaceNumber, UsbBus},
    class_prelude::{DescriptorWriter, EndpointIn, EndpointOut},
};

pub const AUDIO_MAX_PACKET_SIZE: u16 = (96 + 1) * 4 * 2;
const CS_INTERFACE: u8 = 0x24;
const CS_ENDPOINT: u8 = 0x25;

pub fn sample_rate_range() -> [u8; 50] {
    let mut bytes = [0; 50];
    bytes[..2].copy_from_slice(&4_u16.to_le_bytes());
    for (index, rate) in [44_100_u32, 48_000, 88_200, 96_000].into_iter().enumerate() {
        let start = 2 + index * 12;
        bytes[start..start + 4].copy_from_slice(&rate.to_le_bytes());
        bytes[start + 4..start + 8].copy_from_slice(&rate.to_le_bytes());
    }
    bytes
}

pub fn volume_range() -> [u8; 8] {
    let mut bytes = [0; 8];
    bytes[..2].copy_from_slice(&1_u16.to_le_bytes());
    bytes[2..4].copy_from_slice(&Volume::MIN.db_256().to_le_bytes());
    bytes[4..6].copy_from_slice(&Volume::MAX.db_256().to_le_bytes());
    bytes[6..8].copy_from_slice(&Volume::RESOLUTION.to_le_bytes());
    bytes
}

pub fn write_uac2<B: UsbBus>(
    writer: &mut DescriptorWriter,
    control: InterfaceNumber,
    streaming: InterfaceNumber,
    audio_out: &EndpointOut<'_, B>,
    feedback_in: &EndpointIn<'_, B>,
) -> Result<()> {
    writer.iad(control, 2, 0x01, 0x00, 0x20, None)?;
    writer.interface(control, 0x01, 0x01, 0x20)?;

    // Class-specific AC descriptors: header, clock, input, output, feature.
    writer.write(CS_INTERFACE, &[0x01, 0x00, 0x02, 0x01, 0x40, 0x00, 0x00])?;
    writer.write(CS_INTERFACE, &[0x0a, 0x04, 0x03, 0x03, 0x01, 0x00])?;
    writer.write(
        CS_INTERFACE,
        &[
            0x02, 0x01, 0x01, 0x01, 0x00, 0x04, 0x02, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00,
        ],
    )?;
    writer.write(
        CS_INTERFACE,
        &[0x03, 0x03, 0x04, 0x03, 0x01, 0x00, 0x02, 0x04, 0x00, 0x00],
    )?;
    writer.write(
        CS_INTERFACE,
        &[
            0x06, 0x02, 0x01, 0x0f, 0x00, 0x00, 0x00, 0x0f, 0x00, 0x00, 0x00, 0x0f, 0x00, 0x00,
            0x00, 0x00,
        ],
    )?;

    writer.interface_alt(streaming, 0, 0x01, 0x02, 0x20, None)?;
    for (alt, subslot, resolution) in [(1, 2, 16), (2, 4, 24), (3, 4, 32)] {
        writer.interface_alt(streaming, alt, 0x01, 0x02, 0x20, None)?;
        writer.write(
            CS_INTERFACE,
            &[
                0x01, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x02, 0x03, 0x00, 0x00, 0x00, 0x00,
            ],
        )?;
        writer.write(CS_INTERFACE, &[0x02, 0x01, subslot, resolution])?;
        writer.endpoint(audio_out)?;
        writer.write(CS_ENDPOINT, &[0x01, 0x00, 0x00, 0x00, 0x00])?;
        writer.endpoint(feedback_in)?;
    }
    Ok(())
}

pub const HID_REPORT_DESCRIPTOR: &[u8] = &[
    0x06, 0x00, 0xff, 0x09, 0x01, 0xa1, 0x01, 0x09, 0x02, 0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08,
    0x95, 0x10, 0x81, 0x02, 0x09, 0x03, 0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x95, 0x10, 0x91,
    0x02, 0xc0,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_ranges_match_pico_dac2() {
        let rates = sample_rate_range();
        assert_eq!(&rates[..2], &4_u16.to_le_bytes());
        assert_eq!(&rates[2..6], &44_100_u32.to_le_bytes());
        assert_eq!(&rates[38..42], &96_000_u32.to_le_bytes());
        assert_eq!(volume_range(), [1, 0, 0, 160, 0, 0, 0, 1]);
    }

    #[test]
    fn hid_is_vendor_defined_16_byte_in_out() {
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
