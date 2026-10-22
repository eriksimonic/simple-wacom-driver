//! buttons.yaml: what each button does, and the pressure curve.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    VK_BACK, VK_CONTROL, VK_DELETE, VK_ESCAPE, VK_F1, VK_LWIN, VK_MENU, VK_OEM_4, VK_OEM_6, VK_OEM_MINUS,
    VK_OEM_PLUS, VK_RETURN, VK_SHIFT, VK_SPACE, VK_TAB,
};

use crate::tablet::PenState;

const CONFIG_FILE: &str = "buttons.yaml";
/// Tablet buttons are reported as one bit each in a byte, so no tablet can have more than this.
const MAX_TABLET_BUTTONS: usize = u8::BITS as usize;

#[derive(Clone, PartialEq)]
pub enum Action {
    None,
    Barrel,
    Eraser,
    /// Keys are pressed in order and released in reverse.
    Keys(Vec<VIRTUAL_KEY>),
}

/// What the two side buttons on the pen do.
#[derive(Clone)]
pub struct PenButtons {
    pub lower: Action,
    pub upper: Action,
}

impl PenButtons {
    /// True when a pen button that is held down is set to `wanted` in the config.
    pub fn held(&self, pen: &PenState, wanted: &Action) -> bool {
        (pen.lower_button && self.lower == *wanted) || (pen.upper_button && self.upper == *wanted)
    }
}

/// A pressure curve exponent for one app, matched on its lowercase exe name.
#[derive(Clone)]
pub struct AppGamma {
    pub exe: String,
    pub gamma: f32,
}

/// Pressure curve exponent: below 1 boosts light pressure, above 1 softens it, 1 is linear.
#[derive(Clone)]
pub struct Pressure {
    pub gamma: f32,
    /// Overrides `gamma` while that app is in the foreground.
    pub apps: Vec<AppGamma>,
}

pub struct Buttons {
    /// Buttons on the tablet, in the order the tablet numbers them.
    pub tablet: Vec<Action>,
    pub pen: PenButtons,
    pub pressure: Pressure,
}

fn parse_key(name: &str) -> Option<VIRTUAL_KEY> {
    let name = name.trim().to_ascii_lowercase();
    let vk = match name.as_str() {
        "ctrl" => VK_CONTROL.0,
        "shift" => VK_SHIFT.0,
        "alt" => VK_MENU.0,
        "win" => VK_LWIN.0,
        "space" => VK_SPACE.0,
        "tab" => VK_TAB.0,
        "enter" => VK_RETURN.0,
        "esc" => VK_ESCAPE.0,
        "backspace" => VK_BACK.0,
        "delete" => VK_DELETE.0,
        "[" => VK_OEM_4.0,
        "]" => VK_OEM_6.0,
        "plus" => VK_OEM_PLUS.0,
        "minus" => VK_OEM_MINUS.0,
        f if f.starts_with('f') => match f[1..].parse::<u16>() {
            Ok(n) if (1..=12).contains(&n) => VK_F1.0 + n - 1,
            _ => return None,
        },
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
pub fn config_path() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(CONFIG_FILE)))
        .filter(|path| path.exists())
        .unwrap_or_else(|| CONFIG_FILE.into())
}

pub fn load_buttons() -> Buttons {
    let keys = |s: &str| parse_action(s).unwrap();
    let mut tablet = vec![Action::None; MAX_TABLET_BUTTONS];
    tablet[..4].clone_from_slice(&[keys("b"), keys("e"), keys("ctrl+alt+z"), keys("ctrl+shift+z")]);
    let mut buttons = Buttons {
        tablet,
        pen: PenButtons { lower: Action::Barrel, upper: Action::Eraser },
        pressure: Pressure { gamma: 1.2, apps: Vec::new() },
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
                Some(exe) => buttons.pressure.apps.push(AppGamma { exe: exe.to_ascii_lowercase(), gamma }),
                None => buttons.pressure.gamma = gamma,
            }
            continue;
        }
        let action = parse_action(value).unwrap_or_else(|| panic!("{CONFIG_FILE} line {}: bad action '{}'", i + 1, value.trim()));
        match name {
            "pen_lower" => buttons.pen.lower = action,
            "pen_upper" => buttons.pen.upper = action,
            other => match other.strip_prefix("tablet").and_then(|n| n.parse::<usize>().ok()) {
                Some(n) if (1..=MAX_TABLET_BUTTONS).contains(&n) => buttons.tablet[n - 1] = action,
                _ => panic!("{CONFIG_FILE} line {}: unknown button '{other}'", i + 1),
            },
        }
    }
    buttons
}

pub fn run_action(action: &Action) {
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
