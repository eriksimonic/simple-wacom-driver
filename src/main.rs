mod tray;

use rusb::{Direction, TransferType};
use std::time::Duration;

const WACOM_VID: u16 = 0x056A;
const CTL6100WL_PID: u16 = 0x0378;

use windows::Win32::Foundation::POINT;
use windows::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
use windows::Win32::UI::Controls::{
    CreateSyntheticPointerDevice, HSYNTHETICPOINTERDEVICE, POINTER_FEEDBACK_NONE,
    POINTER_TYPE_INFO, POINTER_TYPE_INFO_0,
};
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::Pointer::{
    InjectSyntheticPointerInput, POINTER_FLAGS, POINTER_FLAG_DOWN, POINTER_FLAG_FIRSTBUTTON, POINTER_FLAG_INCONTACT,
    POINTER_FLAG_INRANGE, POINTER_FLAG_NEW, POINTER_FLAG_SECONDBUTTON, POINTER_FLAG_UPDATE, POINTER_FLAG_UP,
    POINTER_INFO, POINTER_PEN_INFO,
};
use windows::core::PWSTR;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{
    GetCurrentThread, OpenProcess, QueryFullProcessImageNameW, SetThreadPriority, PROCESS_NAME_FORMAT,
    PROCESS_QUERY_LIMITED_INFORMATION, THREAD_PRIORITY_HIGHEST,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetSystemMetrics, GetWindowThreadProcessId, PEN_FLAG_BARREL, PEN_FLAG_ERASER, PEN_FLAG_INVERTED, PEN_MASK_PRESSURE, PT_PEN, SM_CXSCREEN, SM_CYSCREEN,
};

const PEN_REPORT: u8 = 0x10;
const TABLET_REPORT: u8 = 0x11;
const PEN_BARREL_BIT: u8 = 0x02;
const PEN_ERASER_BIT: u8 = 0x04;

const CONFIG_FILE: &str = "buttons.yaml";

#[derive(Clone, PartialEq)]
enum Action {
    None,
    Barrel,
    Eraser,
    /// Keys are pressed in order and released in reverse.
    Keys(Vec<VIRTUAL_KEY>),
}

struct Buttons {
    /// Top to bottom.
    tablet: [Action; 4],
    pen_lower: Action,
    pen_upper: Action,
    /// Pressure curve exponent: below 1 boosts light pressure, above 1 softens it, 1 is linear.
    pressure_gamma: f32,
    /// Lowercase exe name and gamma, overriding `pressure_gamma` while that app is in the foreground.
    app_gamma: Vec<(String, f32)>,
}

fn parse_key(name: &str) -> Option<VIRTUAL_KEY> {
    let name = name.trim().to_ascii_lowercase();
    let vk = match name.as_str() {
        "ctrl" => VK_CONTROL.0,
        "shift" => VK_SHIFT.0,
        "alt" => VK_MENU.0,
        "win" => 0x5B,
        "space" => 0x20,
        "tab" => 0x09,
        "enter" => 0x0D,
        "esc" => 0x1B,
        "backspace" => 0x08,
        "delete" => 0x2E,
        "[" => 0xDB,
        "]" => 0xDD,
        "plus" => 0xBB,
        "minus" => 0xBD,
        f if f.starts_with('f') && f[1..].parse::<u16>().is_ok_and(|n| (1..=12).contains(&n)) => {
            0x70 + f[1..].parse::<u16>().unwrap() - 1
        }
        c if c.len() == 1 && c.as_bytes()[0].is_ascii_alphanumeric() => c.to_ascii_uppercase().as_bytes()[0] as u16,
        _ => return None,
    };
    Some(VIRTUAL_KEY(vk))
}

fn parse_action(value: &str) -> Option<Action> {
    match value.trim().trim_matches(|c| c == '"' || c == '\'').to_ascii_lowercase().as_str() {
        "none" => Some(Action::None),
        "barrel" => Some(Action::Barrel),
        "eraser" => Some(Action::Eraser),
        keys => keys.split('+').map(parse_key).collect::<Option<Vec<_>>>().map(Action::Keys),
    }
}

/// The config next to the exe if there is one, so the tool works when started from any folder.
fn config_path() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(CONFIG_FILE)))
        .filter(|path| path.exists())
        .unwrap_or_else(|| CONFIG_FILE.into())
}

fn load_buttons() -> Buttons {
    let keys = |s: &str| parse_action(s).unwrap();
    let mut buttons = Buttons {
        tablet: [keys("b"), keys("e"), keys("ctrl+alt+z"), keys("ctrl+shift+z")],
        pen_lower: Action::Barrel,
        pen_upper: Action::Eraser,
        pressure_gamma: 1.2,
        app_gamma: Vec::new(),
    };
    let Ok(text) = std::fs::read_to_string(config_path()) else {
        println!("no {CONFIG_FILE}, using default buttons");
        return buttons;
    };
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, value) = line.split_once(':').unwrap_or_else(|| panic!("{CONFIG_FILE} line {}: expected name: value", i + 1));
        let name = name.trim();
        if name == "pressure_gamma" || name.starts_with("pressure_gamma.") {
            let gamma = value
                .trim()
                .parse()
                .ok()
                .filter(|g: &f32| *g > 0.0)
                .unwrap_or_else(|| panic!("{CONFIG_FILE} line {}: gamma must be a number above 0", i + 1));
            match name.strip_prefix("pressure_gamma.") {
                Some(exe) => buttons.app_gamma.push((exe.to_ascii_lowercase(), gamma)),
                None => buttons.pressure_gamma = gamma,
            }
            continue;
        }
        let action = parse_action(value).unwrap_or_else(|| panic!("{CONFIG_FILE} line {}: bad action '{}'", i + 1, value.trim()));
        match name {
            "tablet1" => buttons.tablet[0] = action,
            "tablet2" => buttons.tablet[1] = action,
            "tablet3" => buttons.tablet[2] = action,
            "tablet4" => buttons.tablet[3] = action,
            "pen_lower" => buttons.pen_lower = action,
            "pen_upper" => buttons.pen_upper = action,
            other => panic!("{CONFIG_FILE} line {}: unknown button '{other}'", i + 1),
        }
    }
    buttons
}

fn run_action(action: &Action) {
    if let Action::Keys(keys) = action {
        press_keys(keys);
    }
}

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

/// Lowercase exe file name of the foreground window's process, with the window handle for change detection.
fn foreground_exe() -> (isize, Option<String>) {
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return (hwnd.0 as isize, None);
        };
        let mut path = [0u16; 520];
        let mut len = path.len() as u32;
        let queried = QueryFullProcessImageNameW(process, PROCESS_NAME_FORMAT(0), PWSTR(path.as_mut_ptr()), &mut len);
        let _ = CloseHandle(process);
        let exe = queried.ok().and_then(|_| {
            let path = String::from_utf16_lossy(&path[..len as usize]);
            path.rsplit('\\').next().map(str::to_ascii_lowercase)
        });
        (hwnd.0 as isize, exe)
    }
}

/// Maps tablet coordinates onto the primary screen, using the part of the tablet that has the screen's aspect ratio.
/// The screen size is read on every call, so resolution and display scale changes apply without a restart.
struct Mapper;

impl Mapper {
    fn new() -> Self {
        // per-monitor aware, so GetSystemMetrics always returns real pixels, also after a scale change
        let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        Mapper
    }

    fn screen(&self) -> (i32, i32) {
        unsafe { (GetSystemMetrics(SM_CXSCREEN).max(2), GetSystemMetrics(SM_CYSCREEN).max(2)) }
    }

    fn map(&self, x: i32, y: i32) -> (i32, i32) {
        let screen = self.screen();
        let (width, height) = if MAX_X * screen.1 <= MAX_Y * screen.0 {
            (MAX_X, MAX_X * screen.1 / screen.0)
        } else {
            (MAX_Y * screen.0 / screen.1, MAX_Y)
        };
        let (left, top) = ((MAX_X - width) / 2, (MAX_Y - height) / 2);
        (
            ((x - left) * screen.0 / width).clamp(0, screen.0 - 1),
            ((y - top) * screen.1 / height).clamp(0, screen.1 - 1),
        )
    }
}

/// Standard mouse mode: moves the normal cursor, tip is left click, barrel button is right click. No pressure.
struct MouseInjector {
    mapper: Mapper,
    tip: bool,
    barrel: bool,
    last_report: [u8; 10],
    pen_buttons: [(u8, Action); 2],
}

impl MouseInjector {
    fn new(pen_buttons: [(u8, Action); 2]) -> Self {
        MouseInjector { mapper: Mapper::new(), tip: false, barrel: false, last_report: [0; 10], pen_buttons }
    }

    fn send(&mut self, r: &[u8]) {
        if r[..10] == self.last_report {
            return;
        }
        self.last_report.copy_from_slice(&r[..10]);
        let in_range = r[1] & 0x20 != 0;
        let tip = in_range && r[1] & 0x01 != 0;
        let barrel = self.pen_buttons.iter().any(|(bit, action)| r[1] & bit != 0 && *action == Action::Barrel);
        if !in_range && !self.tip && !self.barrel {
            return;
        }

        let x = u16::from_le_bytes([r[2], r[3]]) as i32;
        let y = u16::from_le_bytes([r[5], r[6]]) as i32;
        let (px, py) = self.mapper.map(x, y);
        let (width, height) = self.mapper.screen();
        let mouse = |dx: i32, dy: i32, flags| INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, dwFlags: flags, ..Default::default() } },
        };
        let mut inputs = vec![mouse(px * 65535 / (width - 1), py * 65535 / (height - 1), MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_MOVE)];
        if tip != self.tip {
            inputs.push(mouse(0, 0, if tip { MOUSEEVENTF_LEFTDOWN } else { MOUSEEVENTF_LEFTUP }));
        }
        if barrel != self.barrel {
            inputs.push(mouse(0, 0, if barrel { MOUSEEVENTF_RIGHTDOWN } else { MOUSEEVENTF_RIGHTUP }));
        }
        self.tip = tip;
        self.barrel = barrel;
        unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    }
}

/// Sends the pen to Windows as a synthetic pen, with pressure.
struct Injector {
    device: HSYNTHETICPOINTERDEVICE,
    mapper: Mapper,
    last_flags: POINTER_FLAGS,
    last_report: [u8; 10],
    pen_buttons: [(u8, Action); 2],
    default_gamma: f32,
    app_gamma: Vec<(String, f32)>,
    pressure_gamma: f32,
    foreground: isize,
}

impl Injector {
    fn new(pen_buttons: [(u8, Action); 2], default_gamma: f32, app_gamma: Vec<(String, f32)>) -> Self {
        let device = unsafe { CreateSyntheticPointerDevice(PT_PEN, 1, POINTER_FEEDBACK_NONE) }
            .expect("CreateSyntheticPointerDevice");
        Injector {
            device,
            mapper: Mapper::new(),
            last_flags: POINTER_FLAGS(0),
            last_report: [0; 10],
            pen_buttons,
            default_gamma,
            app_gamma,
            pressure_gamma: default_gamma,
            foreground: 0,
        }
    }

    /// Picks the gamma for the foreground app. Looks up the process only when the foreground window changes.
    fn update_gamma(&mut self) {
        let hwnd = unsafe { GetForegroundWindow() }.0 as isize;
        if hwnd == self.foreground {
            return;
        }
        let (hwnd, exe) = foreground_exe();
        self.foreground = hwnd;
        // popups and system windows can't be read, so keep the gamma of the app they belong to
        let Some(exe) = exe else {
            return;
        };
        let override_gamma = self.app_gamma.iter().find(|(name, _)| *name == exe);
        self.pressure_gamma = override_gamma.map_or(self.default_gamma, |(_, gamma)| *gamma);
        println!("foreground {exe}, pressure_gamma {}", self.pressure_gamma);
    }

    fn send(&mut self, r: &[u8]) {
        if r[..10] == self.last_report {
            return;
        }
        self.last_report.copy_from_slice(&r[..10]);
        self.update_gamma();
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

        let held = |wanted: Action| self.pen_buttons.iter().any(|(bit, action)| r[1] & bit != 0 && *action == wanted);
        let barrel = held(Action::Barrel);
        let eraser = held(Action::Eraser);
        let mut pen_flags = 0u32;
        let mut pointer_flags = flags;
        if barrel {
            pen_flags |= PEN_FLAG_BARREL;
            pointer_flags |= POINTER_FLAG_SECONDBUTTON;
        }
        if eraser {
            pen_flags |= PEN_FLAG_INVERTED | PEN_FLAG_ERASER;
        }

        let (px, py) = self.mapper.map(x, y);
        let info = POINTER_TYPE_INFO {
            r#type: PT_PEN,
            Anonymous: POINTER_TYPE_INFO_0 {
                penInfo: POINTER_PEN_INFO {
                    pointerInfo: POINTER_INFO {
                        pointerType: PT_PEN,
                        pointerId: 0,
                        pointerFlags: pointer_flags,
                        ptPixelLocation: POINT { x: px, y: py },
                        ..Default::default()
                    },
                    penFlags: pen_flags,
                    penMask: PEN_MASK_PRESSURE,
                    pressure: ((pressure as f32 / MAX_PRESSURE as f32).powf(self.pressure_gamma) * WINDOWS_MAX_PRESSURE as f32) as u32,
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

/// Opens the tablet and returns the handle with its interrupt IN endpoint and packet size.
fn open_tablet() -> Option<(rusb::DeviceHandle<rusb::GlobalContext>, u8, u16)> {
    let handle = rusb::open_device_with_vid_pid(WACOM_VID, CTL6100WL_PID)?;
    let config = handle.device().active_config_descriptor().ok()?;
    let (interface, endpoint, size) = config
        .interfaces()
        .flat_map(|iface| iface.descriptors().collect::<Vec<_>>())
        .flat_map(|desc| {
            desc.endpoint_descriptors()
                .map(move |ep| (desc.interface_number(), ep))
                .collect::<Vec<_>>()
        })
        .find(|(_, ep)| ep.direction() == Direction::In && ep.transfer_type() == TransferType::Interrupt)
        .map(|(interface, ep)| (interface, ep.address(), ep.max_packet_size()))?;
    handle.claim_interface(interface).ok()?;
    Some((handle, endpoint, size))
}

fn main() {
    let mode = std::env::args().nth(1);
    let print_only = mode.as_deref() == Some("print");
    let buttons_only = mode.as_deref() == Some("buttons");
    let mouse_mode = mode.as_deref() == Some("mouse");
    let buttons = load_buttons();
    // started from a logon task or a double-click, the console exists only for us, so remove its window
    if mode.is_none() && unsafe { GetConsoleProcessList(&mut [0u32; 2]) } == 1 {
        let _ = unsafe { FreeConsole() };
    }
    if !print_only && !buttons_only {
        tray::spawn(if mouse_mode { "mouse" } else { "pen" }, config_path());
    }
    let pen_buttons = || [(PEN_BARREL_BIT, buttons.pen_lower.clone()), (PEN_ERASER_BIT, buttons.pen_upper.clone())];
    let mut mouse = mouse_mode.then(|| MouseInjector::new(pen_buttons()));
    let mut injector = (!print_only && !mouse_mode)
        .then(|| Injector::new(pen_buttons(), buttons.pressure_gamma, buttons.app_gamma.clone()));
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) }.ok();

    let mut waiting = false;
    loop {
        tray::STATE.store(tray::WAITING, std::sync::atomic::Ordering::Relaxed);
        let Some((handle, endpoint, size)) = open_tablet() else {
            if !waiting {
                println!("waiting for the tablet (is it plugged in and bound to WinUSB?)");
                waiting = true;
            }
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };
        waiting = false;
        tray::STATE.store(tray::CONNECTED, std::sync::atomic::Ordering::Relaxed);
        println!("tablet connected, reading endpoint {endpoint:02x} (ctrl+c to stop)");

        let mut last_seen: std::collections::HashMap<u8, Vec<u8>> = Default::default();
        let mut tablet_buttons = 0u8;
        let mut pen_buttons = 0u8;
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
                    for (i, action) in buttons.tablet.iter().enumerate() {
                        if pressed & (1 << i) != 0 {
                            run_action(action);
                        }
                    }
                }
                Ok(n) if n >= 10 && buf[0] == PEN_REPORT => {
                    let pressed = buf[1] & !pen_buttons;
                    pen_buttons = buf[1];
                    if pressed & PEN_BARREL_BIT != 0 {
                        run_action(&buttons.pen_lower);
                    }
                    if pressed & PEN_ERASER_BIT != 0 {
                        run_action(&buttons.pen_upper);
                    }
                    match (injector.as_mut(), mouse.as_mut()) {
                        (Some(i), _) => i.send(&buf[..n]),
                        (_, Some(m)) => m.send(&buf[..n]),
                        _ => print_pen(&buf[..n]),
                    }
                }
                Ok(_) => {}
                Err(rusb::Error::Timeout) => {}
                Err(e) => {
                    println!("tablet disconnected: {e}");
                    // lift the pen so no stroke or click stays stuck
                    let away = [PEN_REPORT, 0, 0, 0, 0, 0, 0, 0, 0, 0];
                    match (injector.as_mut(), mouse.as_mut()) {
                        (Some(i), _) => i.send(&away),
                        (_, Some(m)) => m.send(&away),
                        _ => {}
                    }
                    break;
                }
            }
        }
    }
}
