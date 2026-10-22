use rusb::{Direction, TransferType};
use std::time::Duration;

const WACOM_VID: u16 = 0x056A;
const CTL6100WL_PID: u16 = 0x0378;

use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Controls::{
    CreateSyntheticPointerDevice, HSYNTHETICPOINTERDEVICE, POINTER_FEEDBACK_DEFAULT,
    POINTER_TYPE_INFO, POINTER_TYPE_INFO_0,
};
use windows::Win32::UI::Input::Pointer::{
    InjectSyntheticPointerInput, POINTER_FLAGS, POINTER_FLAG_DOWN, POINTER_FLAG_FIRSTBUTTON, POINTER_FLAG_INCONTACT,
    POINTER_FLAG_INRANGE, POINTER_FLAG_NEW, POINTER_FLAG_UPDATE, POINTER_FLAG_UP,
    POINTER_INFO, POINTER_PEN_INFO,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, PEN_FLAG_NONE, PEN_MASK_PRESSURE, PT_PEN, SM_CXSCREEN, SM_CYSCREEN,
};

const PEN_REPORT: u8 = 0x10;
const MAX_PRESSURE: u16 = 4095;
const MAX_X: i32 = 21600;
const MAX_Y: i32 = 13500;
const WINDOWS_MAX_PRESSURE: u32 = 1024;

/// Sends the pen to Windows as a synthetic pen, with the tablet area stretched over the primary screen.
struct Injector {
    device: HSYNTHETICPOINTERDEVICE,
    screen: (i32, i32),
    last_flags: POINTER_FLAGS,
}

impl Injector {
    fn new() -> Self {
        let device = unsafe { CreateSyntheticPointerDevice(PT_PEN, 1, POINTER_FEEDBACK_DEFAULT) }
            .expect("CreateSyntheticPointerDevice");
        let screen = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
        Injector { device, screen, last_flags: POINTER_FLAGS(0) }
    }

    fn send(&mut self, r: &[u8]) {
        let in_range = r[1] & 0x20 != 0;
        let tip = r[1] & 0x01 != 0;
        let x = u16::from_le_bytes([r[2], r[3]]) as i32;
        let y = u16::from_le_bytes([r[5], r[6]]) as i32;
        let pressure = u16::from_le_bytes([r[8], r[9]]) as u32;

        let was_in_range = self.last_flags.0 & POINTER_FLAG_INRANGE.0 != 0;
        let was_down = self.last_flags.0 & POINTER_FLAG_INCONTACT.0 != 0;
        let flags = match (in_range, tip, was_in_range, was_down) {
            (false, _, false, _) => return,
            (false, _, true, _) => POINTER_FLAG_UPDATE,
            (true, true, _, false) => {
                POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT | POINTER_FLAG_FIRSTBUTTON | POINTER_FLAG_DOWN
            }
            (true, true, _, true) => {
                POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT | POINTER_FLAG_FIRSTBUTTON | POINTER_FLAG_UPDATE
            }
            (true, false, _, true) => POINTER_FLAG_INRANGE | POINTER_FLAG_UP,
            (true, false, false, false) => POINTER_FLAG_NEW | POINTER_FLAG_INRANGE | POINTER_FLAG_UPDATE,
            (true, false, true, false) => POINTER_FLAG_INRANGE | POINTER_FLAG_UPDATE,
        };
        self.last_flags = flags;

        let info = POINTER_TYPE_INFO {
            r#type: PT_PEN,
            Anonymous: POINTER_TYPE_INFO_0 {
                penInfo: POINTER_PEN_INFO {
                    pointerInfo: POINTER_INFO {
                        pointerType: PT_PEN,
                        pointerId: 0,
                        pointerFlags: flags,
                        ptPixelLocation: POINT {
                            x: x * self.screen.0 / MAX_X,
                            y: y * self.screen.1 / MAX_Y,
                        },
                        ..Default::default()
                    },
                    penFlags: PEN_FLAG_NONE,
                    penMask: PEN_MASK_PRESSURE,
                    pressure: pressure * WINDOWS_MAX_PRESSURE / MAX_PRESSURE as u32,
                    ..Default::default()
                },
            },
        };
        let _ = unsafe { InjectSyntheticPointerInput(self.device, &[info]) };
    }
}

fn print_pen(r: &[u8]) {
    let in_range = r[1] & 0x20 != 0;
    let tip = r[1] & 0x01 != 0;
    let x = u16::from_le_bytes([r[2], r[3]]);
    let y = u16::from_le_bytes([r[5], r[6]]);
    let pressure = u16::from_le_bytes([r[8], r[9]]);
    let bar = "#".repeat((pressure as usize * 40) / MAX_PRESSURE as usize);
    println!("range={} tip={} x={x:5} y={y:5} pressure={pressure:4} {bar}", in_range as u8, tip as u8);
}

fn main() {
    let handle = rusb::open_device_with_vid_pid(WACOM_VID, CTL6100WL_PID)
        .expect("tablet not found or not bound to WinUSB");
    let config = handle.device().active_config_descriptor().expect("config descriptor");

    let mut target = None;
    for iface in config.interfaces() {
        for desc in iface.descriptors() {
            for ep in desc.endpoint_descriptors() {
                println!(
                    "interface {} endpoint {:02x} {:?} {:?} max_packet={}",
                    desc.interface_number(),
                    ep.address(),
                    ep.direction(),
                    ep.transfer_type(),
                    ep.max_packet_size()
                );
                if ep.direction() == Direction::In
                    && ep.transfer_type() == TransferType::Interrupt
                    && target.is_none()
                {
                    target = Some((desc.interface_number(), ep.address(), ep.max_packet_size()));
                }
            }
        }
    }

    let (interface, endpoint, size) = target.expect("no interrupt IN endpoint");
    handle.claim_interface(interface).expect("claim interface");
    println!("reading endpoint {endpoint:02x}, move the pen (ctrl+c to stop)");

    let print_only = std::env::args().nth(1).as_deref() == Some("print");
    let mut injector = (!print_only).then(Injector::new);
    let mut buf = vec![0u8; size as usize];
    loop {
        match handle.read_interrupt(endpoint, &mut buf, Duration::from_millis(500)) {
            Ok(n) if n >= 10 && buf[0] == PEN_REPORT => match injector.as_mut() {
                Some(i) => i.send(&buf[..n]),
                None => print_pen(&buf[..n]),
            },
            Ok(_) => {}
            Err(rusb::Error::Timeout) => {}
            Err(e) => panic!("read: {e}"),
        }
    }
}
