# Adding a tablet

A tablet is one entry in `src/tablet.rs`: USB ids, where each value sits in the reports, and the value ranges. The `probe` mode collects everything needed to write that entry.

## Project layout

| File | Job |
|---|---|
| `src/tablet.rs` | Tablet profiles and report decoding. This is the file to edit for a new tablet. |
| `src/output.rs` | Turns decoded pen state into a Windows Ink pen or a mouse. Tablet independent. |
| `src/config.rs` | `buttons.yaml`. Tablet independent. |
| `src/main.rs` | Finds a known tablet, reads it, routes reports. |
| `src/probe.rs` | The `probe` mode. |
| `src/tray.rs` | Tray icon. |

## 1. Find the USB id

Device Manager, right-click the tablet, Properties, Details, Hardware Ids. It looks like `USB\VID_056A&PID_0378`. The two hex numbers are the vendor id and the product id.

## 2. Bind the tablet to WinUSB

Run Zadig as administrator (it is in the installer folder, or from https://zadig.akeo.ie). Options > List All Devices, pick the tablet, set the driver to WinUSB, click Replace Driver. The tablet stops working with its normal driver until you undo this: Device Manager, Uninstall device, tick "Delete the driver software", replug.

## 3. Record the tablet

```
wacom.exe probe 056a:0378
```

Use your own id. For a Wacom tablet the id is optional. It takes about two minutes.

1. Enter how many buttons are on the tablet itself.
2. For each gesture, read the instruction, press Enter, and do it until the timer ends. Type `s` and Enter to skip a gesture the tablet or pen does not have.
3. Do each gesture clearly and repeat it. Press one button at a time. Use the full surface for the hover and corner gestures, and go from the lightest to the hardest pressure for the pressure gesture.

The tool writes `tablet-probe-<vid>-<pid>.md` in the current folder. It records no serial number.

## 4. Send the file

Open an issue at https://github.com/eriksimonic/simple-wacom-driver/issues and attach the file. Add:

- the tablet model and the pen model
- whether the tablet is wired or wireless
- the Windows version and screen resolution

## If no reports arrived

The probe prints a warning and the file says so. The tablet is silent until something switches it into its full reporting mode. To find that command:

1. Install Wireshark with USBPcap, and bind nothing yet. Keep the manufacturer driver installed.
2. Capture the tablet's USB bus, then unplug and replug the tablet so the driver initialises it.
3. Look for control transfers from the host to the tablet just after enumeration, with `bmRequestType 0x21` and `bRequest 0x09` (HID SET_REPORT). Note the `wValue` and the data bytes.
4. Add them to the `init` list of the tablet entry. `wValue` is `0x0300 + report id` for a feature report and `0x0200 + report id` for an output report.

## Writing the entry yourself

Copy `WACOM_INTUOS_M_CTL_6100WL` in `src/tablet.rs`, add it to `TABLETS`, and fill in the values from the probe file.

| Field | Where to find it |
|---|---|
| `vid`, `pid` | The probe header. |
| `pen.id`, `pen.length` | The capture of the hover gesture. The report id is the first byte. |
| `x`, `y`, `pressure` offsets and sizes | The HID report descriptor lines for the pen report. Add up the bit sizes in order, 8 bits per byte, to get each byte offset. Confirm with the "Bytes that changed" lines in the hover and pressure captures. Values are little-endian. |
| `max_x`, `max_y`, `max_pressure` | The `logical` maximum in the descriptor lines. The corner capture shows the largest values reached. The ratio of `max_x` to `max_y` must match the tablet's real shape. |
| `flags` and the bit masks | The one byte that changes between idle, hover, touch and the button captures. Compare it by hand: tip down, in range, lower button, upper button. |
| `buttons` | The report that changes only in the tablet button captures. Bit 0 is button 1. |

Then check it:

```
cargo run -- print     # position and pressure should move, pressure should reach max_pressure
cargo run -- buttons   # each tablet and pen button should print one line on press and release
```

If the layout does not fit the structs in `tablet.rs` (big-endian values, a pen split over several reports, tilt you want to use), extend `tablet.rs`. `output.rs` only needs `PenState`.
