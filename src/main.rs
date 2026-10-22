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
    POINTER_FLAG_INRANGE, POINTER_FLAG_NEW, POINTER_FLAG_SECONDBUTTON, POINTER_FLAG_UPDATE, POINTER_FLAG_UP,
    POINTER_INFO, POINTER_PEN_INFO,
};
use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, PEN_FLAG_BARREL, PEN_FLAG_ERASER, PEN_FLAG_INVERTED, PEN_MASK_PRESSURE, PT_PEN, SM_CXSCREEN, SM_CYSCREEN,
};

const PEN_REPORT: u8 = 0x10;
const TABLET_REPORT: u8 = 0x11;
const PEN_BARREL_BIT: u8 = 0x02;
const PEN_ERASER_BIT: u8 = 0x04;

/// Shortcut for each of the 4 tablet buttons, top to bottom. Keys are pressed in order and released in reverse.
const TABLET_KEYS: [&[VIRTUAL_KEY]; 4] = [
    &[VIRTUAL_KEY(b'B' as u16)],
    &[VIRTUAL_KEY(b'E' as u16)],
    &[VK_CONTROL, VK_MENU, VIRTUAL_KEY(b'Z' as u16)],
    &[VK_CONTROL, VK_SHIFT, VIRTUAL_KEY(b'Z' as u16)],
];

fn press_keys(keys: &[VIRTUAL_KEY]) {
    let key = |vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, dwFlags: flags, ..Default::default() } },
    };
    let inputs: Vec<INPUT> = keys
        .iter()
        .map(|&vk| key(vk, KEYBD_EVENT_FLAGS(0)))
        .chain(keys.iter().rev().map(|&vk| key(vk, KEYEVENTF_KEYUP)))
        .collect();
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
}

const MAX_PRESSURE: u16 = 4095;
const MAX_X: i32 = 21600;
const MAX_Y: i32 = 13500;
const WINDOWS_MAX_PRESSURE: u32 = 1024;

/// Sends the pen to Windows as a synthetic pen, with the tablet area stretched over the primary screen.
struct Injector {
    device: HSYNTHETICPOINTERDEVICE,
    screen: (i32, i32),
    last_flags: POINTER_FLAGS,
    last_report: [u8; 10],
}

impl Injector {
    fn new() -> Self {
        let device = unsafe { CreateSyntheticPointerDevice(PT_PEN, 1, POINTER_FEEDBACK_DEFAULT) }
            .expect("CreateSyntheticPointerDevice");
        let screen = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
        Injector { device, screen, last_flags: POINTER_FLAGS(0), last_report: [0; 10] }
    }

    fn send(&mut self, r: &[u8]) {
        if r[..10] == self.last_report {
            return;
        }
        self.last_report.copy_from_slice(&r[..10]);
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

        let barrel = r[1] & PEN_BARREL_BIT != 0;
        let eraser = r[1] & PEN_ERASER_BIT != 0;
        let mut pen_flags = 0u32;
        let mut pointer_flags = flags;
        if barrel {
            pen_flags |= PEN_FLAG_BARREL;
            pointer_flags |= POINTER_FLAG_SECONDBUTTON;
        }
        if eraser {
            pen_flags |= PEN_FLAG_INVERTED | PEN_FLAG_ERASER;
        }

        let info = POINTER_TYPE_INFO {
            r#type: PT_PEN,
            Anonymous: POINTER_TYPE_INFO_0 {
                penInfo: POINTER_PEN_INFO {
                    pointerInfo: POINTER_INFO {
                        pointerType: PT_PEN,
                        pointerId: 0,
                        pointerFlags: pointer_flags,
                        ptPixelLocation: POINT {
                            x: x * self.screen.0 / MAX_X,
                            y: y * self.screen.1 / MAX_Y,
                        },
                        ..Default::default()
                    },
                    penFlags: pen_flags,
                    penMask: PEN_MASK_PRESSURE,
                    pressure: pressure * WINDOWS_MAX_PRESSURE / MAX_PRESSURE as u32,
                    ..Default::default()
                },
            },
        };
        let _ = unsafe { InjectSyntheticPointerInput(self.device, &[info]) };
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
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

    let mode = std::env::args().nth(1);
    let print_only = mode.as_deref() == Some("print");
    let buttons_only = mode.as_deref() == Some("buttons");
    let mut last_seen: std::collections::HashMap<u8, Vec<u8>> = Default::default();
    let mut injector = (!print_only).then(Injector::new);
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) }.ok();
    let mut tablet_buttons = 0u8;
    let mut buf = vec![0u8; size as usize];
    loop {
        match handle.read_interrupt(endpoint, &mut buf, Duration::from_millis(500)) {
            Ok(n) if buttons_only => {
                // pen reports differ constantly in position, so compare only the flags byte
                let key = if buf[0] == PEN_REPORT { vec![buf[1]] } else { buf[..n].to_vec() };
                if last_seen.get(&buf[0]) != Some(&key) {
                    println!("{}", hex(&buf[..n]));
                    last_seen.insert(buf[0], key);
                }
            }
            Ok(n) if n >= 2 && buf[0] == TABLET_REPORT && !print_only => {
                let pressed = buf[1] & !tablet_buttons;
                tablet_buttons = buf[1];
                for (i, keys) in TABLET_KEYS.iter().enumerate() {
                    if pressed & (1 << i) != 0 {
                        press_keys(keys);
                    }
                }
            }
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
