//! Taskbar tray icon: green when the tablet is connected, grey while waiting for it.
//! Runs on its own thread with a hidden window and message loop. Right-click for the menu.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU8, Ordering};
use std::sync::OnceLock;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    ShellExecuteW, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics, PostMessageW, RegisterClassW,
    RegisterWindowMessageW, SetForegroundWindow, SetTimer, TrackPopupMenu, TranslateMessage, HICON, ICONINFO,
    MF_GRAYED, MF_STRING, MSG, SM_CXSMICON, SW_SHOWNORMAL, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WM_APP,
    WM_COMMAND, WM_NULL, WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_OVERLAPPED,
};

#[derive(Clone, Copy, PartialEq)]
pub enum State {
    Waiting,
    Connected,
}

/// Written by the reader loop, read by the tray timer. Holds a `State` discriminant.
static STATE: AtomicU8 = AtomicU8::new(State::Waiting as u8);

pub fn set_state(state: State) {
    STATE.store(state as u8, Ordering::Relaxed);
}

fn state() -> State {
    if STATE.load(Ordering::Relaxed) == State::Connected as u8 { State::Connected } else { State::Waiting }
}

const CALLBACK: u32 = WM_APP + 1;
const MENU_OPEN_CONFIG: usize = 1;
const MENU_EXIT: usize = 2;
const TIMER_ID: usize = 1;
const NOT_SHOWN: u32 = u32::MAX;
const NO_BATTERY: u8 = 255;

static ICONS: [AtomicIsize; 2] = [AtomicIsize::new(0), AtomicIsize::new(0)];
static ADDED: AtomicBool = AtomicBool::new(false);
static SHOWN: AtomicU32 = AtomicU32::new(NOT_SHOWN);
/// Battery percent, or `NO_BATTERY` when unknown. Written by the reader loop.
static BATTERY: AtomicU8 = AtomicU8::new(NO_BATTERY);
static CHARGING: AtomicBool = AtomicBool::new(false);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static INFO: OnceLock<(String, std::path::PathBuf)> = OnceLock::new();

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Draws a filled circle with a white dot in the middle, as a 32-bit icon.
fn make_icon(size: i32, (r, g, b): (u8, u8, u8)) -> HICON {
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    let centre = (size as f32 - 1.0) / 2.0;
    for y in 0..size {
        for x in 0..size {
            let distance = ((x as f32 - centre).powi(2) + (y as f32 - centre).powi(2)).sqrt();
            let i = ((y * size + x) * 4) as usize;
            let (b, g, r) = if distance <= size as f32 * 0.14 { (255, 255, 255) } else { (b, g, r) };
            if distance <= size as f32 / 2.0 - 0.5 {
                pixels[i..i + 4].copy_from_slice(&[b, g, r, 255]);
            }
        }
    }
    // the mask is ignored for 32-bit colour with alpha, but CreateIconIndirect still needs one
    let mask = vec![0u8; (((size + 15) / 16) * 2 * size) as usize];
    unsafe {
        let color = CreateBitmap(size, size, 1, 32, Some(pixels.as_ptr() as *const c_void));
        let mask = CreateBitmap(size, size, 1, 1, Some(mask.as_ptr() as *const c_void));
        let icon = CreateIconIndirect(&ICONINFO { fIcon: true.into(), hbmMask: mask, hbmColor: color, ..Default::default() })
            .expect("CreateIconIndirect");
        let _ = DeleteObject(color);
        let _ = DeleteObject(mask);
        icon
    }
}

pub fn set_battery(percent: u8, charging: bool) {
    BATTERY.store(percent, Ordering::Relaxed);
    CHARGING.store(charging, Ordering::Relaxed);
}

pub fn clear_battery() {
    BATTERY.store(NO_BATTERY, Ordering::Relaxed);
}

fn tooltip(state: State) -> String {
    let mode = &INFO.get().unwrap().0;
    let status = if state == State::Connected { "tablet connected" } else { "waiting for the tablet" };
    let battery = match BATTERY.load(Ordering::Relaxed) {
        NO_BATTERY => String::new(),
        percent if CHARGING.load(Ordering::Relaxed) => format!(", battery {percent}%, charging"),
        percent => format!(", battery {percent}%"),
    };
    format!("Simple Wacom driver ({mode} mode): {status}{battery}")
}

/// Everything the tooltip shows, as one number, to detect a change.
fn shown_signature(state: State) -> u32 {
    state as u32 | (BATTERY.load(Ordering::Relaxed) as u32) << 8 | (CHARGING.load(Ordering::Relaxed) as u32) << 16
}

fn notify(hwnd: HWND, action: windows::Win32::UI::Shell::NOTIFY_ICON_MESSAGE, state: State) -> bool {
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: CALLBACK,
        hIcon: HICON(ICONS[state as usize].load(Ordering::Relaxed) as *mut c_void),
        ..Default::default()
    };
    let tip = wide(&tooltip(state));
    let len = tip.len().min(data.szTip.len() - 1);
    data.szTip[..len].copy_from_slice(&tip[..len]);
    unsafe { Shell_NotifyIconW(action, &data) }.as_bool()
}

/// Adds the icon, or updates it when the state changed. Also retries the add, because the task can start
/// before Explorer has created the tray.
fn refresh(hwnd: HWND) {
    let state = state();
    if !ADDED.load(Ordering::Relaxed) {
        if notify(hwnd, NIM_ADD, state) {
            ADDED.store(true, Ordering::Relaxed);
            SHOWN.store(shown_signature(state), Ordering::Relaxed);
        }
    } else if SHOWN.load(Ordering::Relaxed) != shown_signature(state) {
        notify(hwnd, NIM_MODIFY, state);
        SHOWN.store(shown_signature(state), Ordering::Relaxed);
    }
}

fn show_menu(hwnd: HWND) {
    let status = wide(&tooltip(state()));
    let open = wide("Open buttons.yaml");
    let exit = wide("Exit");
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, PCWSTR(status.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, MENU_OPEN_CONFIG, PCWSTR(open.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, MENU_EXIT, PCWSTR(exit.as_ptr()));
        let mut at = POINT::default();
        let _ = GetCursorPos(&mut at);
        // the menu only closes when clicking elsewhere if our window is in the foreground
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, at.x, at.y, 0, hwnd, None);
        let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_TIMER => refresh(hwnd),
        CALLBACK if lparam.0 as u32 == WM_RBUTTONUP => show_menu(hwnd),
        WM_COMMAND => match wparam.0 & 0xFFFF {
            MENU_OPEN_CONFIG => {
                let path = wide(&INFO.get().unwrap().1.to_string_lossy());
                ShellExecuteW(hwnd, w!("open"), w!("notepad.exe"), PCWSTR(path.as_ptr()), None, SW_SHOWNORMAL);
            }
            MENU_EXIT => {
                notify(hwnd, NIM_DELETE, state());
                std::process::exit(0);
            }
            _ => {}
        },
        // Explorer restarted, so the icon is gone
        m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            ADDED.store(false, Ordering::Relaxed);
            refresh(hwnd);
        }
        _ => return DefWindowProcW(hwnd, message, wparam, lparam),
    }
    LRESULT(0)
}

/// Starts the tray thread. `mode` is shown in the tooltip, `config` is opened by the menu.
pub fn spawn(mode: &str, config: std::path::PathBuf) {
    let _ = INFO.set((mode.to_string(), config));
    std::thread::spawn(|| unsafe {
        let size = GetSystemMetrics(SM_CXSMICON).max(16);
        ICONS[State::Waiting as usize].store(make_icon(size, (140, 140, 140)).0 as isize, Ordering::Relaxed);
        ICONS[State::Connected as usize].store(make_icon(size, (40, 180, 70)).0 as isize, Ordering::Relaxed);
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);

        let instance = GetModuleHandleW(None).expect("GetModuleHandleW");
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("SimpleWacomTray"),
            ..Default::default()
        };
        RegisterClassW(&class);
        // a normal hidden window, not message-only, so it receives the TaskbarCreated broadcast
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("SimpleWacomTray"),
            w!("Simple Wacom driver"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            instance,
            None,
        )
        .expect("CreateWindowExW");
        refresh(hwnd);
        SetTimer(hwnd, TIMER_ID, 500, None);

        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    });
}
