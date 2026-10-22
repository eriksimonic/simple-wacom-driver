//! buttons.yaml: what each button does, and the pressure curve.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    VK_CONTROL, VK_MENU, VK_SHIFT,
};

const CONFIG_FILE: &str = "buttons.yaml";
const TABLET_BUTTONS: usize = 8;

#[derive(Clone, PartialEq)]
pub enum Action {
    None,
    Barrel,
    Eraser,
    /// Keys are pressed in order and released in reverse.
    Keys(Vec<VIRTUAL_KEY>),
}

pub struct Buttons {
    /// Buttons on the tablet, in the order the tablet numbers them.
    pub tablet: Vec<Action>,
    pub pen_lower: Action,
    pub pen_upper: Action,
    /// Pressure curve exponent: below 1 boosts light pressure, above 1 softens it, 1 is linear.
    pub pressure_gamma: f32,
    /// Lowercase exe name and gamma, overriding `pressure_gamma` while that app is in the foreground.
    pub app_gamma: Vec<(String, f32)>,
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
pub fn config_path() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(CONFIG_FILE)))
        .filter(|path| path.exists())
        .unwrap_or_else(|| CONFIG_FILE.into())
}

pub fn load_buttons() -> Buttons {
    let keys = |s: &str| parse_action(s).unwrap();
    let mut tablet = vec![Action::None; TABLET_BUTTONS];
    tablet[..4].clone_from_slice(&[keys("b"), keys("e"), keys("ctrl+alt+z"), keys("ctrl+shift+z")]);
    let mut buttons = Buttons {
        tablet,
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
            "pen_lower" => buttons.pen_lower = action,
            "pen_upper" => buttons.pen_upper = action,
            other => match other.strip_prefix("tablet").and_then(|n| n.parse::<usize>().ok()) {
                Some(n) if (1..=TABLET_BUTTONS).contains(&n) => buttons.tablet[n - 1] = action,
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
