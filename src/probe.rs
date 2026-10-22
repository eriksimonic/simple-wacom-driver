//! `wacom.exe probe`: guided capture of what is needed to add support for a new tablet.
//! Writes a markdown report with the USB descriptors, the HID report descriptor and the raw reports
//! for each gesture. See docs/adding-a-tablet.md.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

use rusb::{Direction, Recipient, RequestType, TransferType};

/// Name, what to do, seconds to record.
const STEPS: &[(&str, &str, u64)] = &[
    ("idle", "Keep the pen away from the tablet and do not touch any button.", 4),
    ("hover", "Hold the pen about 5 mm above the surface and move it slowly over the whole active area. Do not touch the surface.", 8),
    ("corners", "Touch the pen tip to each corner in turn: top-left, top-right, bottom-right, bottom-left. Hold each for one second, then lift the pen.", 12),
    ("pressure", "Draw a line while pressing from the lightest touch to the hardest press, then release. Repeat a few times.", 10),
    ("tilt", "Touch the surface and tilt the pen to all four sides, keeping the tip down.", 8),
    ("pen button 1", "Hover the pen and press and release the lower side button a few times.", 6),
    ("pen button 2", "Hover the pen and press and release the upper side button a few times.", 6),
    ("eraser", "Flip the pen and move the eraser end over the surface, touching a few times.", 6),
    ("other controls", "Use any touch ring, strip, wheel or touch surface a few times.", 6),
];

#[derive(Default)]
struct Stats {
    len: usize,
    count: u32,
    min: Vec<u8>,
    max: Vec<u8>,
    samples: Vec<Vec<u8>>,
}

impl Stats {
    fn add(&mut self, report: &[u8]) {
        if self.count == 0 {
            self.len = report.len();
            self.min = report.to_vec();
            self.max = report.to_vec();
        } else {
            for (i, &byte) in report.iter().enumerate().take(self.len) {
                self.min[i] = self.min[i].min(byte);
                self.max[i] = self.max[i].max(byte);
            }
        }
        self.count += 1;
        if self.samples.len() < 10 && !self.samples.iter().any(|s| s == report) {
            self.samples.push(report.to_vec());
        }
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

/// The first interrupt IN endpoint: interface number, address and max packet size.
pub fn find_endpoint(config: &rusb::ConfigDescriptor) -> Option<(u8, u8, u16)> {
    config
        .interfaces()
        .flat_map(|iface| iface.descriptors().collect::<Vec<_>>())
        .flat_map(|desc| {
            desc.endpoint_descriptors()
                .map(move |ep| (desc.interface_number(), ep))
                .collect::<Vec<_>>()
        })
        .find(|(_, ep)| ep.direction() == Direction::In && ep.transfer_type() == TransferType::Interrupt)
        .map(|(interface, ep)| (interface, ep.address(), ep.max_packet_size()))
}

fn ask(prompt: &str) -> String {
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    line.trim().to_ascii_lowercase()
}

fn parse_id(arg: &str) -> Option<(u16, u16)> {
    let (vid, pid) = arg.split_once(':')?;
    Some((u16::from_str_radix(vid, 16).ok()?, u16::from_str_radix(pid, 16).ok()?))
}

/// Lists the main items of a HID report descriptor as one line each.
fn describe_descriptor(descriptor: &[u8]) -> String {
    let mut out = String::new();
    let (mut usage_page, mut report_id, mut size, mut count) = (0u32, 0u32, 0u32, 0u32);
    let (mut logical_min, mut logical_max) = (0i64, 0i64);
    let mut usages: Vec<u32> = Vec::new();
    let mut i = 0;
    while i < descriptor.len() {
        let prefix = descriptor[i];
        if prefix == 0xFE {
            // long item, not used by real devices
            i += 3 + descriptor.get(i + 1).copied().unwrap_or(0) as usize;
            continue;
        }
        let len = [0usize, 1, 2, 4][(prefix & 3) as usize];
        let data = descriptor.get(i + 1..i + 1 + len).unwrap_or(&[]);
        let unsigned = data.iter().rev().fold(0u32, |acc, &b| (acc << 8) | b as u32);
        let signed = match len {
            1 => unsigned as u8 as i8 as i64,
            2 => unsigned as u16 as i16 as i64,
            _ => unsigned as i32 as i64,
        };
        let (kind, tag) = ((prefix >> 2) & 3, prefix >> 4);
        match (kind, tag) {
            (1, 0) => usage_page = unsigned,
            (1, 1) => logical_min = signed,
            (1, 2) => logical_max = if logical_min < 0 { signed } else { unsigned as i64 },
            (1, 7) => size = unsigned,
            (1, 8) => report_id = unsigned,
            (1, 9) => count = unsigned,
            (2, 0) => usages.push(unsigned),
            (2, 1) | (2, 2) => usages.push(unsigned),
            (0, 8 | 9 | 11) => {
                let name = match tag {
                    8 => "input",
                    9 => "output",
                    _ => "feature",
                };
                let usage_list = usages.iter().map(|u| format!("{u:#x}")).collect::<Vec<_>>().join(", ");
                let constant = if unsigned & 1 != 0 { " (constant)" } else { "" };
                let _ = writeln!(
                    out,
                    "- report {report_id:#04x} {name}{constant}: usage page {usage_page:#x}, usages [{usage_list}], {size} bits x {count}, logical {logical_min}..{logical_max}"
                );
            }
            _ => {}
        }
        if kind == 0 {
            usages.clear();
        }
        i += 1 + len;
    }
    out
}

fn markdown_table(stats: &BTreeMap<u8, Stats>) -> String {
    let mut out = String::new();
    for (id, s) in stats {
        let _ = writeln!(out, "\nReport id `{id:#04x}`: {} bytes, {} received.\n", s.len, s.count);
        let varying: Vec<String> = (0..s.len)
            .filter(|&i| s.min[i] != s.max[i])
            .map(|i| format!("byte {i}: {:#04x}..{:#04x}", s.min[i], s.max[i]))
            .collect();
        let _ = writeln!(out, "Bytes that changed: {}\n", if varying.is_empty() { "none".into() } else { varying.join("; ") });
        let _ = writeln!(out, "```\n{}\n```", s.samples.iter().map(|r| hex(r)).collect::<Vec<_>>().join("\n"));
    }
    out
}

pub fn run(args: &[String]) {
    println!("Tablet probe. It records what your tablet sends so support for it can be added.");
    println!("The tablet must be bound to the WinUSB driver (see docs/adding-a-tablet.md).\n");

    let wanted = args.first().and_then(|a| parse_id(a));
    let devices = rusb::devices().expect("list USB devices");
    let device = devices
        .iter()
        .find(|d| {
            d.device_descriptor()
                .map(|desc| match wanted {
                    Some((vid, pid)) => desc.vendor_id() == vid && desc.product_id() == pid,
                    None => desc.vendor_id() == 0x056A,
                })
                .unwrap_or(false)
        })
        .unwrap_or_else(|| {
            println!("No matching tablet found. Pass its USB ID as vid:pid in hex, for example: wacom.exe probe 056a:0378");
            println!("USB devices seen:");
            for d in devices.iter() {
                if let Ok(desc) = d.device_descriptor() {
                    println!("  {:04x}:{:04x}", desc.vendor_id(), desc.product_id());
                }
            }
            std::process::exit(1);
        });
    let descriptor = device.device_descriptor().expect("device descriptor");
    let (vid, pid) = (descriptor.vendor_id(), descriptor.product_id());
    let handle = device.open().unwrap_or_else(|e| {
        println!("Cannot open {vid:04x}:{pid:04x}: {e}. Bind it to WinUSB with Zadig first.");
        std::process::exit(1);
    });
    let product = handle.read_product_string_ascii(&descriptor).unwrap_or_default();
    let manufacturer = handle.read_manufacturer_string_ascii(&descriptor).unwrap_or_default();
    println!("Found {vid:04x}:{pid:04x} {manufacturer} {product}");

    let mut report = String::new();
    let _ = writeln!(report, "# Tablet probe {vid:04x}:{pid:04x}\n");
    let _ = writeln!(report, "- tool version: {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(report, "- USB id: `{vid:04x}:{pid:04x}`, device version {}", descriptor.device_version());
    let _ = writeln!(report, "- manufacturer: {manufacturer}, product: {product}");

    let config = device.active_config_descriptor().expect("config descriptor");
    let mut hid_interface = None;
    let _ = writeln!(report, "\n## Interfaces and endpoints\n");
    for iface in config.interfaces() {
        for desc in iface.descriptors() {
            let _ = writeln!(report, "- interface {}: class {:#04x}, subclass {:#04x}", desc.interface_number(), desc.class_code(), desc.sub_class_code());
            if desc.class_code() == 3 && hid_interface.is_none() {
                hid_interface = Some(desc.interface_number());
            }
            for ep in desc.endpoint_descriptors() {
                let _ = writeln!(report, "  - endpoint {:#04x} {:?} {:?}, max packet {}", ep.address(), ep.direction(), ep.transfer_type(), ep.max_packet_size());
            }
        }
    }
    let (interface, endpoint, size) = find_endpoint(&config).expect("no interrupt IN endpoint found");
    handle.claim_interface(interface).expect("claim interface");

    if let Some(iface) = hid_interface {
        let mut buf = vec![0u8; 4096];
        let request = rusb::request_type(Direction::In, RequestType::Standard, Recipient::Interface);
        match handle.read_control(request, 0x06, 0x2200, iface as u16, &mut buf, Duration::from_secs(1)) {
            Ok(n) => {
                let _ = writeln!(report, "\n## HID report descriptor ({n} bytes)\n\n```\n{}\n```\n\n{}", hex(&buf[..n]), describe_descriptor(&buf[..n]));
            }
            Err(e) => {
                let _ = writeln!(report, "\nHID report descriptor unavailable: {e}");
            }
        }
    }

    println!("\nYou will do a series of gestures. Each one is recorded for a few seconds.");
    println!("Type s then Enter to skip a gesture the tablet or pen does not have.\n");

    let tablet_buttons: usize = ask("How many buttons are on the tablet itself? (0 if none) ").parse().unwrap_or(0);
    let mut steps: Vec<(String, String, u64)> = STEPS.iter().map(|(n, t, s)| (n.to_string(), t.to_string(), *s)).collect();
    for i in 1..=tablet_buttons {
        steps.insert(
            steps.len() - 1,
            (format!("tablet button {i}"), format!("Press and release tablet button {i} (counted from the top or left) a few times. Touch nothing else."), 5),
        );
    }

    let _ = writeln!(report, "\n## Captures");
    let mut total = 0u32;
    let mut buf = vec![0u8; size as usize];
    for (name, text, seconds) in &steps {
        println!("--- {name} ---\n{text}");
        if ask(&format!("Press Enter, then do it for {seconds} seconds (s to skip): ")) == "s" {
            let _ = writeln!(report, "\n### {name}\n\nSkipped.");
            continue;
        }
        let mut stats: BTreeMap<u8, Stats> = BTreeMap::new();
        let deadline = Instant::now() + Duration::from_secs(*seconds);
        let mut error = None;
        while Instant::now() < deadline {
            match handle.read_interrupt(endpoint, &mut buf, Duration::from_millis(50)) {
                Ok(n) if n > 0 => stats.entry(buf[0]).or_default().add(&buf[..n]),
                Ok(_) | Err(rusb::Error::Timeout) => {}
                Err(e) => {
                    error = Some(e);
                    break;
                }
            }
        }
        let received: u32 = stats.values().map(|s| s.count).sum();
        total += received;
        println!("recorded {received} reports\n");
        let _ = writeln!(report, "\n### {name}\n\nInstruction: {text}");
        if let Some(e) = error {
            let _ = writeln!(report, "\nRead error: {e}");
        }
        report.push_str(&markdown_table(&stats));
    }

    if total == 0 {
        println!("No reports arrived at all. The tablet probably needs an init command; see docs/adding-a-tablet.md.");
        let _ = writeln!(report, "\n**No reports arrived during the whole session.**");
    }
    let path = format!("tablet-probe-{vid:04x}-{pid:04x}.md");
    std::fs::write(&path, report).expect("write report");
    println!("Wrote {path}. Attach it to a GitHub issue with the tablet model and pen model.");
}
