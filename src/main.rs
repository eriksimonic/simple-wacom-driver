mod config;
mod output;
mod probe;
mod tablet;
mod tray;

use std::sync::atomic::Ordering;
use std::time::Duration;

use rusb::{Direction, Recipient, RequestType, TransferType};
use windows::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST};

use config::{config_path, load_buttons, run_action};
use output::{Injector, MouseInjector};
use tablet::{PenState, Tablet, TABLETS};

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
        let set_report = rusb::request_type(Direction::Out, RequestType::Class, Recipient::Interface);
        for init in tablet.init {
            handle
                .write_control(set_report, 0x09, init.value, interface as u16, init.data, Duration::from_secs(1))
                .ok()?;
        }
        Some(Connection { handle, endpoint, size, tablet })
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
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
    let mode = args.first().cloned();
    if mode.as_deref() == Some("probe") {
        probe::run(&args[1..]);
        return;
    }
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
    let mut mouse = mouse_mode.then(|| MouseInjector::new(buttons.pen_lower.clone(), buttons.pen_upper.clone()));
    let mut injector = (!print_only && !mouse_mode).then(|| {
        Injector::new(buttons.pen_lower.clone(), buttons.pen_upper.clone(), buttons.pressure_gamma, buttons.app_gamma.clone())
    });
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) }.ok();

    let mut waiting = false;
    loop {
        tray::STATE.store(tray::WAITING, Ordering::Relaxed);
        let Some(Connection { handle, endpoint, size, tablet }) = open_tablet() else {
            if !waiting {
                println!("waiting for a tablet (is it plugged in and bound to WinUSB?)");
                waiting = true;
            }
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };
        waiting = false;
        tray::STATE.store(tray::CONNECTED, Ordering::Relaxed);
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

            if buttons_only {
                // pen reports differ constantly in position, so compare only the flags byte
                let key = if report[0] == tablet.pen.id { vec![report[tablet.pen.flags]] } else { report.to_vec() };
                if last_seen.get(&report[0]) != Some(&key) {
                    println!("{}", hex(report));
                    last_seen.insert(report[0], key);
                }
            } else if let Some(mask) = tablet.parse_buttons(report).filter(|_| !print_only) {
                let pressed = mask & !tablet_buttons;
                tablet_buttons = mask;
                for (i, action) in buttons.tablet.iter().enumerate() {
                    if pressed & (1 << i) != 0 {
                        run_action(action);
                    }
                }
            } else if let Some(pen) = tablet.parse_pen(report) {
                if !print_only && pen.lower_button && !lower_down {
                    run_action(&buttons.pen_lower);
                }
                if !print_only && pen.upper_button && !upper_down {
                    run_action(&buttons.pen_upper);
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
