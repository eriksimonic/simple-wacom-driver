use rusb::{Direction, TransferType};
use std::time::Duration;

const WACOM_VID: u16 = 0x056A;
const CTL6100WL_PID: u16 = 0x0378;

const PEN_REPORT: u8 = 0x10;
const MAX_PRESSURE: u16 = 4095;

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

    let mut buf = vec![0u8; size as usize];
    loop {
        match handle.read_interrupt(endpoint, &mut buf, Duration::from_millis(500)) {
            Ok(n) if n >= 10 && buf[0] == PEN_REPORT => print_pen(&buf[..n]),
            Ok(_) => {}
            Err(rusb::Error::Timeout) => {}
            Err(e) => panic!("read: {e}"),
        }
    }
}
