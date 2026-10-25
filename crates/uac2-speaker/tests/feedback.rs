use std::sync::{Arc, Mutex};

use audio_core::{AudioEngine, FeedbackQ16, SampleFormat, SampleRate, StreamConfig};
use uac2_speaker::Uac2Speaker;
use usb_device::{
    Result, UsbDirection, UsbError,
    bus::{PollResult, UsbBus, UsbBusAllocator},
    device::{UsbDeviceBuilder, UsbVidPid},
    endpoint::{EndpointAddress, EndpointType},
};

const FEEDBACK_COMPLETE: u32 = 1 << 2;
const AUDIO_OUT: u32 = 1 << 3;

#[derive(Default)]
struct Registers {
    status: u32,
    setup: Option<[u8; 8]>,
    reset: bool,
    feedback_available: bool,
    feedback_writes: Vec<[u8; 4]>,
    complete_feedback_after_snapshot: bool,
}

impl Registers {
    fn set_alt(&mut self, alt: u8) {
        // SET_INTERFACE for the AudioStreaming interface (interface 1).
        self.setup = Some([0x01, 0x0b, alt, 0, 1, 0, 0, 0]);
    }

    fn complete_feedback(&mut self) {
        assert!(self.feedback_available);
        self.feedback_available = false;
        self.status |= FEEDBACK_COMPLETE;
    }
}

struct TestBus(Arc<Mutex<Registers>>);

impl UsbBus for TestBus {
    fn alloc_ep(
        &mut self,
        dir: UsbDirection,
        addr: Option<EndpointAddress>,
        _: EndpointType,
        _: u16,
        _: u8,
    ) -> Result<EndpointAddress> {
        Ok(addr.unwrap_or_else(|| EndpointAddress::from_parts(0, dir)))
    }

    fn enable(&mut self) {}

    fn reset(&self) {
        let mut r = self.0.lock().unwrap();
        r.status = 0;
        r.setup = None;
        r.feedback_available = false;
    }

    fn set_device_address(&self, _: u8) {}
    fn set_stalled(&self, _: EndpointAddress, _: bool) {}
    fn is_stalled(&self, _: EndpointAddress) -> bool {
        false
    }
    fn suspend(&self) {}
    fn resume(&self) {}

    fn write(&self, addr: EndpointAddress, bytes: &[u8]) -> Result<usize> {
        let mut r = self.0.lock().unwrap();
        if addr.index() == 1 {
            if r.feedback_available {
                return Err(UsbError::WouldBlock);
            }
            r.feedback_available = true;
            r.feedback_writes.push(bytes.try_into().unwrap());
        }
        Ok(bytes.len())
    }

    fn read(&self, addr: EndpointAddress, output: &mut [u8]) -> Result<usize> {
        let mut r = self.0.lock().unwrap();
        if addr.index() == 0 {
            let setup = r.setup.take().ok_or(UsbError::WouldBlock)?;
            output[..setup.len()].copy_from_slice(&setup);
            Ok(setup.len())
        } else if r.status & AUDIO_OUT != 0 {
            r.status &= !AUDIO_OUT;
            output[..48 * 4].fill(0);
            Ok(48 * 4)
        } else {
            Err(UsbError::WouldBlock)
        }
    }

    fn poll(&self) -> PollResult {
        let mut r = self.0.lock().unwrap();
        if std::mem::take(&mut r.reset) {
            return PollResult::Reset;
        }
        let snapshot = r.status;
        let setup = r.setup.is_some();
        if snapshot == 0 && !setup {
            return PollResult::None;
        }

        // rp2040-hal 0.11.0 reads BUFF_STATUS, then acknowledges all IN
        // bits. An IN completion between those operations is lost even
        // though the endpoint buffer is now available for another write.
        if std::mem::take(&mut r.complete_feedback_after_snapshot) {
            r.complete_feedback();
        }
        r.status &= !0x5555_5555;

        PollResult::Data {
            ep_out: if snapshot & AUDIO_OUT != 0 { 2 } else { 0 },
            ep_in_complete: if snapshot & FEEDBACK_COMPLETE != 0 {
                2
            } else {
                0
            },
            ep_setup: u16::from(setup),
        }
    }
}

#[test]
fn feedback_updates_once_per_successful_transfer() {
    let r = Arc::new(Mutex::new(Registers::default()));
    let alloc = UsbBusAllocator::new(TestBus(r.clone()));
    let mut speaker = Uac2Speaker::new(&alloc);
    let mut device = UsbDeviceBuilder::new(&alloc, UsbVidPid(0xcafe, 0xbabe)).build();
    let mut audio = AudioEngine::new();
    audio.start(
        StreamConfig {
            rate: SampleRate::Hz48000,
            format: SampleFormat::Pcm16,
        },
        48_000,
    );
    audio.push_usb_packet(&[0; 480 * 4]).unwrap();

    r.lock().unwrap().set_alt(1);
    device.poll(&mut [&mut speaker]);
    let nominal = FeedbackQ16::from_rate_hz(48_000).to_usb_bytes();
    assert_eq!(r.lock().unwrap().feedback_writes, [nominal]);

    for transfer in 1..=100 {
        // Follow main's ordering: poll USB, process audio, then prepare the
        // next feedback value only when requested by a successful write.
        assert!(speaker.feedback_requested());
        let feedback = audio.feedback();
        assert_ne!(feedback.to_usb_bytes(), nominal);
        speaker.set_feedback(feedback);
        assert!(!speaker.feedback_requested());

        // OUT events and idle main-loop iterations must not advance the
        // occupancy filter while the feedback endpoint is still busy.
        for _ in 0..10 {
            r.lock().unwrap().status |= AUDIO_OUT;
            device.poll(&mut [&mut speaker]);
            speaker.take_packet();
            assert!(!speaker.feedback_requested());
            assert_eq!(r.lock().unwrap().feedback_writes.len(), transfer);
            assert!(!device.poll(&mut [&mut speaker]));
            assert!(!speaker.feedback_requested());
        }

        r.lock().unwrap().complete_feedback();
        device.poll(&mut [&mut speaker]);
        assert_eq!(
            r.lock().unwrap().feedback_writes[transfer],
            feedback.to_usb_bytes()
        );
    }
}

#[test]
fn feedback_continues_when_completion_notifications_are_lost() {
    let r = Arc::new(Mutex::new(Registers::default()));
    let alloc = UsbBusAllocator::new(TestBus(r.clone()));
    let mut speaker = Uac2Speaker::new(&alloc);
    let mut device = UsbDeviceBuilder::new(&alloc, UsbVidPid(0xcafe, 0xbabe)).build();
    r.lock().unwrap().set_alt(1);
    device.poll(&mut [&mut speaker]);

    for transfer in 1..=1000 {
        let feedback = FeedbackQ16::from_rate_hz(48_000 + transfer as u32 % 100);
        speaker.set_feedback(feedback);
        {
            let mut r = r.lock().unwrap();
            r.status |= AUDIO_OUT;
            r.complete_feedback_after_snapshot = true;
        }
        device.poll(&mut [&mut speaker]);
        speaker.take_packet();
        let r = r.lock().unwrap();
        assert_eq!(r.feedback_writes.len(), transfer + 1);
        assert_eq!(r.feedback_writes[transfer], feedback.to_usb_bytes());
        assert!(r.feedback_available);
    }
}

#[test]
fn stopping_and_resetting_clear_feedback_updates() {
    let r = Arc::new(Mutex::new(Registers::default()));
    let alloc = UsbBusAllocator::new(TestBus(r.clone()));
    let mut speaker = Uac2Speaker::new(&alloc);
    let mut device = UsbDeviceBuilder::new(&alloc, UsbVidPid(0xcafe, 0xbabe)).build();
    r.lock().unwrap().set_alt(1);
    device.poll(&mut [&mut speaker]);
    speaker.set_feedback(FeedbackQ16::from_rate_hz(48_100));

    {
        let mut r = r.lock().unwrap();
        r.complete_feedback();
        r.set_alt(0);
    }
    device.poll(&mut [&mut speaker]);
    assert!(!speaker.feedback_requested());
    assert_eq!(r.lock().unwrap().feedback_writes.len(), 1);

    r.lock().unwrap().set_alt(1);
    device.poll(&mut [&mut speaker]);
    assert!(speaker.feedback_requested());
    assert_eq!(
        r.lock().unwrap().feedback_writes[1],
        FeedbackQ16::from_rate_hz(48_000).to_usb_bytes()
    );

    r.lock().unwrap().reset = true;
    device.poll(&mut [&mut speaker]);
    assert!(!speaker.feedback_requested());
    r.lock().unwrap().status |= AUDIO_OUT;
    device.poll(&mut [&mut speaker]);
    assert_eq!(r.lock().unwrap().feedback_writes.len(), 2);
}
