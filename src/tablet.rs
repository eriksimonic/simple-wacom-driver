//! Tablet profiles: the USB ids and where each value sits in the reports.
//! To add a tablet, add a `Tablet` to `TABLETS`. See docs/adding-a-tablet.md.

/// An unsigned little-endian value at a byte offset in a report.
pub struct Field {
    pub offset: usize,
    pub bytes: usize,
}

impl Field {
    fn read(&self, report: &[u8]) -> u32 {
        report[self.offset..self.offset + self.bytes].iter().rev().fold(0, |acc, &b| (acc << 8) | b as u32)
    }
}

/// The report that carries the pen position, pressure and pen buttons.
pub struct PenReport {
    pub id: u8,
    pub length: usize,
    /// Byte offset of the flags byte, and the bit masks inside it.
    pub flags: usize,
    pub in_range: u8,
    pub tip: u8,
    pub lower_button: u8,
    pub upper_button: u8,
    pub x: Field,
    pub y: Field,
    pub pressure: Field,
    /// Largest values the tablet reports. The X/Y ratio must match the tablet's real shape.
    pub max_x: i32,
    pub max_y: i32,
    pub max_pressure: u32,
}

/// The report that carries the buttons on the tablet itself, one bit per button, bit 0 is button 1.
pub struct ButtonReport {
    pub id: u8,
    pub offset: usize,
}

/// A HID SET_REPORT sent once after the interface is claimed, for tablets that stay silent until told to report.
/// `value` is `(report type << 8) | report id`: 0x0200 + id for an output report, 0x0300 + id for a feature report.
pub struct InitReport {
    pub value: u16,
    pub data: &'static [u8],
}

pub struct Tablet {
    pub name: &'static str,
    pub vid: u16,
    pub pid: u16,
    pub pen: PenReport,
    pub buttons: Option<ButtonReport>,
    pub init: &'static [InitReport],
}

/// One decoded pen report. X, Y and pressure are in the tablet's own units.
#[derive(Clone, Copy, PartialEq, Default)]
pub struct PenState {
    pub in_range: bool,
    pub tip: bool,
    pub lower_button: bool,
    pub upper_button: bool,
    pub x: i32,
    pub y: i32,
    pub pressure: u32,
}

impl Tablet {
    pub fn parse_pen(&self, report: &[u8]) -> Option<PenState> {
        let pen = &self.pen;
        if report.first() != Some(&pen.id) || report.len() < pen.length {
            return None;
        }
        let flags = report[pen.flags];
        Some(PenState {
            in_range: flags & pen.in_range != 0,
            tip: flags & pen.tip != 0,
            lower_button: flags & pen.lower_button != 0,
            upper_button: flags & pen.upper_button != 0,
            x: pen.x.read(report) as i32,
            y: pen.y.read(report) as i32,
            pressure: pen.pressure.read(report),
        })
    }

    /// Bit mask of the pressed tablet buttons.
    pub fn parse_buttons(&self, report: &[u8]) -> Option<u8> {
        let buttons = self.buttons.as_ref()?;
        if report.first() != Some(&buttons.id) || report.len() <= buttons.offset {
            return None;
        }
        Some(report[buttons.offset])
    }
}

pub const TABLETS: &[Tablet] = &[WACOM_INTUOS_M_CTL_6100WL];

const WACOM_INTUOS_M_CTL_6100WL: Tablet = Tablet {
    name: "Wacom Intuos M (CTL-6100WL)",
    vid: 0x056A,
    pid: 0x0378,
    pen: PenReport {
        id: 0x10,
        length: 10,
        flags: 1,
        in_range: 0x20,
        tip: 0x01,
        lower_button: 0x02,
        upper_button: 0x04,
        x: Field { offset: 2, bytes: 3 },
        y: Field { offset: 5, bytes: 3 },
        pressure: Field { offset: 8, bytes: 2 },
        max_x: 21600,
        max_y: 13500,
        max_pressure: 4095,
    },
    buttons: Some(ButtonReport { id: 0x11, offset: 1 }),
    init: &[],
};
