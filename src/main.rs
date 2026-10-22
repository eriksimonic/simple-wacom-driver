mod config;
mod output;
mod probe;
mod tablet;
mod tray;

use std::time::Duration;

use rusb::{Direction, Recipient, RequestType};
use windows::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST};

use config::{config_path, load_buttons, run_action};
use output::{Injector, MouseInjector};
use tablet::{PenState, Tablet, TABLETS};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Windows Ink pen, the default.
    Pen,
    /// Plain mouse cursor.
    Mouse,
    /// Print the decoded pen state, change nothing.
    Print,
    /// Print each distinct raw report, to find the tablet button report.
    Buttons,
}

/// A tablet that is plugged in, bound to WinUSB and ready to read.
struct Connection {
    handle: rusb::DeviceHandle<rusb::GlobalContext>,
    endpoint: u8,
    size: u16,
    tablet: &'static Tablet,
}

/// Opens the first known tablet that is plugged in and sends its init reports.
fn open_tablet() -> Option<Connection> {
    TABLETS.iter().find_map(|tablet| {
        let handle = rusb::open_device_with_vid_pid(tablet.vid, tablet.pid)?;
        let config = handle.device().active_config_descriptor().ok()?;
        let (interface, endpoint, size) = probe::find_endpoint(&config)?;
        handle.claim_interface(interface).ok()?;
        let set_report = rusb::request_type(Direction::Out, RequestType::Class, Recipient::Interface);
        for init in tablet.init {
            handle
                .write_control(set_report, 0x09, init.value, interface as u16, init.data, Duration::from_secs(1))
                .ok()?;
        }
        Some(Connection { handle, endpoint, size, tablet })
    })
}

fn print_pen(tablet: &Tablet, pen: &PenState) {
    let bar = "#".repeat((pen.pressure as usize * 40) / tablet.pen.max_pressure as usize);
    println!(
        "range={} tip={} x={:5} y={:5} pressure={:4} {bar}",
        pen.in_range as u8, pen.tip as u8, pen.x, pen.y, pen.pressure
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = match args.first().map(String::as_str) {
        Some("probe") => {
            probe::run(&args[1..]);
            return;
        }
        Some("print") => Mode::Print,
        Some("buttons") => Mode::Buttons,
        Some("mouse") => Mode::Mouse,
        _ => Mode::Pen,
    };
    let buttons = load_buttons();
    // started from a logon task or a double-click, the console exists only for us, so remove its window
    if args.is_empty() && unsafe { GetConsoleProcessList(&mut [0u32; 2]) } == 1 {
        let _ = unsafe { FreeConsole() };
    }
    match mode {
        Mode::Pen => tray::spawn("pen", config_path()),
        Mode::Mouse => tray::spawn("mouse", config_path()),
        Mode::Print | Mode::Buttons => {}
    }
    let mut mouse = (mode == Mode::Mouse).then(|| MouseInjector::new(buttons.pen.clone()));
    let mut injector = (mode == Mode::Pen).then(|| Injector::new(buttons.pen.clone(), buttons.pressure.clone()));
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) }.ok();

    let mut waiting = false;
    loop {
        tray::set_state(tray::State::Waiting);
        tray::clear_battery();
        let Some(Connection { handle, endpoint, size, tablet }) = open_tablet() else {
            if !waiting {
                println!("waiting for a tablet (is it plugged in and bound to WinUSB?)");
                waiting = true;
            }
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };
        waiting = false;
        tray::set_state(tray::State::Connected);
        println!("{} connected, reading endpoint {endpoint:02x} (ctrl+c to stop)", tablet.name);

        let mut last_seen: std::collections::HashMap<u8, Vec<u8>> = Default::default();
        let mut tablet_buttons = 0u8;
        let (mut lower_down, mut upper_down) = (false, false);
        let mut buf = vec![0u8; size as usize];
        loop {
            let n = match handle.read_interrupt(endpoint, &mut buf, Duration::from_millis(500)) {
                Ok(n) => n,
                Err(rusb::Error::Timeout) => continue,
                Err(e) => {
                    println!("tablet disconnected: {e}");
                    // lift the pen so no stroke or click stays stuck
                    let away = PenState::default();
                    if let Some(i) = injector.as_mut() {
                        i.send(&tablet.pen, away);
                    }
                    if let Some(m) = mouse.as_mut() {
                        m.send(&tablet.pen, away);
                    }
                    break;
                }
            };
            let report = &buf[..n];
            if report.is_empty() {
                continue;
            }

            if mode == Mode::Buttons {
                // pen reports differ constantly in position, so compare only the flags byte
                let key = if report[0] == tablet.pen.id { vec![report[tablet.pen.flags]] } else { report.to_vec() };
                if last_seen.get(&report[0]) != Some(&key) {
                    println!("{}", probe::hex(report));
                    last_seen.insert(report[0], key);
                }
            } else if let Some(mask) = tablet.parse_buttons(report).filter(|_| mode != Mode::Print) {
                let pressed = mask & !tablet_buttons;
                tablet_buttons = mask;
                for (i, action) in buttons.tablet.iter().enumerate() {
                    if pressed & (1 << i) != 0 {
                        run_action(action);
                    }
                }
            } else if let Some((percent, charging)) = tablet.parse_battery(report) {
                tray::set_battery(percent, charging);
            } else if let Some(pen) = tablet.parse_pen(report) {
                if mode != Mode::Print && pen.lower_button && !lower_down {
                    run_action(&buttons.pen.lower);
                }
                if mode != Mode::Print && pen.upper_button && !upper_down {
                    run_action(&buttons.pen.upper);
                }
                (lower_down, upper_down) = (pen.lower_button, pen.upper_button);
                if let Some(i) = injector.as_mut() {
                    i.send(&tablet.pen, pen);
                } else if let Some(m) = mouse.as_mut() {
                    m.send(&tablet.pen, pen);
                } else {
                    print_pen(tablet, &pen);
                }
            }
        }
    }
}
