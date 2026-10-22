//! Turns pen state into Windows input: a synthetic Windows Ink pen, or a plain mouse.

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, POINT};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Controls::{
    CreateSyntheticPointerDevice, HSYNTHETICPOINTERDEVICE, POINTER_FEEDBACK_NONE, POINTER_TYPE_INFO,
    POINTER_TYPE_INFO_0,
};
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEINPUT,
};
use windows::Win32::UI::Input::Pointer::{
    InjectSyntheticPointerInput, POINTER_FLAGS, POINTER_FLAG_DOWN, POINTER_FLAG_FIRSTBUTTON, POINTER_FLAG_INCONTACT,
    POINTER_FLAG_INRANGE, POINTER_FLAG_NEW, POINTER_FLAG_SECONDBUTTON, POINTER_FLAG_UP, POINTER_FLAG_UPDATE,
    POINTER_INFO, POINTER_PEN_INFO,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetSystemMetrics, GetWindowThreadProcessId, PEN_FLAG_BARREL, PEN_FLAG_ERASER,
    PEN_FLAG_INVERTED, PEN_MASK_PRESSURE, PT_PEN, SM_CXSCREEN, SM_CYSCREEN,
};

use crate::config::Action;
use crate::tablet::{PenReport, PenState};

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

    fn map(&self, layout: &PenReport, pen: &PenState) -> (i32, i32) {
        let screen = self.screen();
        let (max_x, max_y) = (layout.max_x, layout.max_y);
        let (width, height) = if max_x * screen.1 <= max_y * screen.0 {
            (max_x, max_x * screen.1 / screen.0)
        } else {
            (max_y * screen.0 / screen.1, max_y)
        };
        let (left, top) = ((max_x - width) / 2, (max_y - height) / 2);
        (
            ((pen.x - left) * screen.0 / width).clamp(0, screen.0 - 1),
            ((pen.y - top) * screen.1 / height).clamp(0, screen.1 - 1),
        )
    }
}

/// True when a pen button that is held down is set to `wanted` in the config.
fn held(pen: &PenState, lower: &Action, upper: &Action, wanted: &Action) -> bool {
    (pen.lower_button && lower == wanted) || (pen.upper_button && upper == wanted)
}

/// Standard mouse mode: moves the normal cursor, tip is left click, barrel button is right click. No pressure.
pub struct MouseInjector {
    mapper: Mapper,
    tip: bool,
    barrel: bool,
    last: Option<PenState>,
    pen_lower: Action,
    pen_upper: Action,
}

impl MouseInjector {
    pub fn new(pen_lower: Action, pen_upper: Action) -> Self {
        MouseInjector { mapper: Mapper::new(), tip: false, barrel: false, last: None, pen_lower, pen_upper }
    }

    pub fn send(&mut self, layout: &PenReport, pen: PenState) {
        if self.last == Some(pen) {
            return;
        }
        self.last = Some(pen);
        let tip = pen.in_range && pen.tip;
        let barrel = held(&pen, &self.pen_lower, &self.pen_upper, &Action::Barrel);
        if !pen.in_range && !self.tip && !self.barrel {
            return;
        }

        let (px, py) = self.mapper.map(layout, &pen);
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
pub struct Injector {
    device: HSYNTHETICPOINTERDEVICE,
    mapper: Mapper,
    last_flags: POINTER_FLAGS,
    last: Option<PenState>,
    pen_lower: Action,
    pen_upper: Action,
    default_gamma: f32,
    app_gamma: Vec<(String, f32)>,
    pressure_gamma: f32,
    foreground: isize,
}

impl Injector {
    pub fn new(pen_lower: Action, pen_upper: Action, default_gamma: f32, app_gamma: Vec<(String, f32)>) -> Self {
        let device = unsafe { CreateSyntheticPointerDevice(PT_PEN, 1, POINTER_FEEDBACK_NONE) }
            .expect("CreateSyntheticPointerDevice");
        Injector {
            device,
            mapper: Mapper::new(),
            last_flags: POINTER_FLAGS(0),
            last: None,
            pen_lower,
            pen_upper,
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

    pub fn send(&mut self, layout: &PenReport, pen: PenState) {
        if self.last == Some(pen) {
            return;
        }
        self.last = Some(pen);
        self.update_gamma();

        let was_in_range = self.last_flags.0 & POINTER_FLAG_INRANGE.0 != 0;
        let was_down = self.last_flags.0 & POINTER_FLAG_INCONTACT.0 != 0;
        let flags = match (pen.in_range, pen.tip, was_in_range, was_down) {
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

        let mut pen_flags = 0u32;
        let mut pointer_flags = flags;
        if held(&pen, &self.pen_lower, &self.pen_upper, &Action::Barrel) {
            pen_flags |= PEN_FLAG_BARREL;
            pointer_flags |= POINTER_FLAG_SECONDBUTTON;
        }
        if held(&pen, &self.pen_lower, &self.pen_upper, &Action::Eraser) {
            pen_flags |= PEN_FLAG_INVERTED | PEN_FLAG_ERASER;
        }

        let (px, py) = self.mapper.map(layout, &pen);
        let curved = (pen.pressure as f32 / layout.max_pressure as f32).powf(self.pressure_gamma);
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
                    pressure: (curved * WINDOWS_MAX_PRESSURE as f32) as u32,
                    ..Default::default()
                },
            },
        };
        let _ = unsafe { InjectSyntheticPointerInput(self.device, &[info]) };
    }
}
