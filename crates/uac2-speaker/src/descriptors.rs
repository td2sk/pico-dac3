use audio_core::{SampleRate, Volume};
use usb_device::{
    Result,
    bus::{InterfaceNumber, UsbBus},
    class_prelude::{DescriptorWriter, EndpointIn, EndpointOut},
};

pub const AUDIO_MAX_PACKET_SIZE: u16 = (96 + 1) * 4 * 2;
const CS_INTERFACE: u8 = 0x24;
const CS_ENDPOINT: u8 = 0x25;
const RANGE_HEADER_LEN: usize = 2;
const SAMPLE_RATE_SUBRANGE_LEN: usize = 12;

pub const SAMPLE_RATE_RANGE_LEN: usize =
    RANGE_HEADER_LEN + SampleRate::ALL.len() * SAMPLE_RATE_SUBRANGE_LEN;

pub const fn sample_rate_range() -> [u8; SAMPLE_RATE_RANGE_LEN] {
    let mut bytes = [0; SAMPLE_RATE_RANGE_LEN];

    let subrange_count = (SampleRate::ALL.len() as u16).to_le_bytes();
    bytes[0] = subrange_count[0];
    bytes[1] = subrange_count[1];

    let mut index = 0;
    while index < SampleRate::ALL.len() {
        let rate = SampleRate::ALL[index].hz().to_le_bytes();
        let start = RANGE_HEADER_LEN + index * SAMPLE_RATE_SUBRANGE_LEN;

        // dMIN
        bytes[start] = rate[0];
        bytes[start + 1] = rate[1];
        bytes[start + 2] = rate[2];
        bytes[start + 3] = rate[3];

        // dMAX
        bytes[start + 4] = rate[0];
        bytes[start + 5] = rate[1];
        bytes[start + 6] = rate[2];
        bytes[start + 7] = rate[3];

        // dRES remains zero for a discrete sample rate.
        index += 1;
    }

    bytes
}

pub const fn volume_range() -> [u8; 8] {
    let subrange_count = 1_u16.to_le_bytes();
    let min = Volume::MIN.db_256().to_le_bytes();
    let max = Volume::MAX.db_256().to_le_bytes();
    let resolution = Volume::RESOLUTION.to_le_bytes();

    [
        subrange_count[0],
        subrange_count[1],
        min[0],
        min[1],
        max[0],
        max[1],
        resolution[0],
        resolution[1],
    ]
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

    // Class-specific AudioControl header.
    writer.write(
        CS_INTERFACE,
        &[
            0x01, // bDescriptorSubtype: HEADER
            0x00, 0x02, // bcdADC: Audio Device Class 2.0
            0x01, // bCategory: Desktop Speaker
            0x40, 0x00, // wTotalLength: all class-specific AC descriptors
            0x00, // bmControls: no latency control
        ],
    )?;

    // Clock Source entity. Sampling frequency is RW; clock validity is RO.
    writer.write(
        CS_INTERFACE,
        &[
            0x0a, // bDescriptorSubtype: CLOCK_SOURCE
            0x04, // bClockID: CLOCK_SOURCE
            0x03, // bmAttributes: internal programmable clock
            0x07, // bmControls: frequency RW, validity RO
            0x01, // bAssocTerminal: INPUT_TERMINAL
            0x00, // iClockSource: no string
        ],
    )?;

    // USB Streaming Input Terminal.
    writer.write(
        CS_INTERFACE,
        &[
            0x02, // bDescriptorSubtype: INPUT_TERMINAL
            0x01, // bTerminalID: INPUT_TERMINAL
            0x01, 0x01, // wTerminalType: USB Streaming
            0x00, // bAssocTerminal: none
            0x04, // bCSourceID: CLOCK_SOURCE
            0x02, // bNrChannels: stereo
            0x03, 0x00, 0x00, 0x00, // bmChannelConfig: Front Left/Right
            0x00, // iChannelNames: no string
            0x00, 0x00, // bmControls: none
            0x00, // iTerminal: no string
        ],
    )?;

    // Desktop Speaker Output Terminal fed by the Feature Unit.
    writer.write(
        CS_INTERFACE,
        &[
            0x03, // bDescriptorSubtype: OUTPUT_TERMINAL
            0x03, // bTerminalID: OUTPUT_TERMINAL
            0x04, 0x03, // wTerminalType: Desktop Speaker
            0x01, // bAssocTerminal: INPUT_TERMINAL
            0x02, // bSourceID: FEATURE_UNIT
            0x04, // bCSourceID: CLOCK_SOURCE
            0x00, 0x00, // bmControls: none
            0x00, // iTerminal: no string
        ],
    )?;

    // Feature Unit with mute and volume controls for master, left, and right.
    writer.write(
        CS_INTERFACE,
        &[
            0x06, // bDescriptorSubtype: FEATURE_UNIT
            0x02, // bUnitID: FEATURE_UNIT
            0x01, // bSourceID: INPUT_TERMINAL
            0x0f, 0x00, 0x00, 0x00, // bmaControls(0): master mute/volume RW
            0x0f, 0x00, 0x00, 0x00, // bmaControls(1): left mute/volume RW
            0x0f, 0x00, 0x00, 0x00, // bmaControls(2): right mute/volume RW
            0x00, // iFeature: no string
        ],
    )?;

    writer.interface_alt(streaming, 0, 0x01, 0x02, 0x20, None)?;
    for (alt, subslot, resolution) in [(1, 2, 16), (2, 4, 24), (3, 4, 32)] {
        writer.interface_alt(streaming, alt, 0x01, 0x02, 0x20, None)?;

        // Class-specific AudioStreaming general descriptor.
        writer.write(
            CS_INTERFACE,
            &[
                0x01, // bDescriptorSubtype: AS_GENERAL
                0x01, // bTerminalLink: INPUT_TERMINAL
                0x00, // bmControls: none
                0x01, // bFormatType: FORMAT_TYPE_I
                0x01, 0x00, 0x00, 0x00, // bmFormats: PCM
                0x02, // bNrChannels: stereo
                0x03, 0x00, 0x00, 0x00, // bmChannelConfig: Front Left/Right
                0x00, // iChannelNames: no string
            ],
        )?;

        // Type I format descriptor. Subslot size and bit resolution vary by alt.
        writer.write(
            CS_INTERFACE,
            &[
                0x02,       // bDescriptorSubtype: FORMAT_TYPE
                0x01,       // bFormatType: FORMAT_TYPE_I
                subslot,    // bSubslotSize
                resolution, // bBitResolution
            ],
        )?;

        writer.endpoint(audio_out)?;

        // Class-specific isochronous audio data endpoint descriptor.
        writer.write(
            CS_ENDPOINT,
            &[
                0x01, // bDescriptorSubtype: EP_GENERAL
                0x00, // bmAttributes: none
                0x00, // bmControls: none
                0x00, // bLockDelayUnits: undefined
                0x00, 0x00, // wLockDelay: zero
            ],
        )?;

        writer.endpoint(feedback_in)?;
    }
    Ok(())
}

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
