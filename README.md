# simple-wacom-driver

Small Windows tool for a Wacom Intuos (CTL-6100WL, USB `056A:0378`). It reads the tablet over WinUSB and gives pen pressure to Photoshop and Lightroom Classic through Windows Ink. The Wacom driver is not needed.

Supported tablets: Wacom Intuos M (CTL-6100WL). Another tablet is one entry in `src/tablet.rs`, and the `probe` mode records what is needed to write it. See [docs/adding-a-tablet.md](docs/adding-a-tablet.md).

## Why

The official Wacom driver is large and noisy for a tablet that only needs to send position, pressure and a few buttons.

- **Size.** In my experience the Wacom driver runs several background processes and uses hundreds of MB of RAM. This tool is one 0.4 MB exe. Measured on the release build with the tray icon and the tablet connected: 4.6 MB private memory, 11.9 MB working set (that figure includes shared Windows DLL pages).
- **Popups.** The Wacom driver shows popups. This tool shows none. The only UI is a tray icon, green when the tablet is connected and grey when it is waiting.
- **Control.** Button shortcuts, the pressure curve and per-app curves are one small text file.

The trade-off is that this is a hobby tool for one model. It has no pen tilt, no multi-monitor mapping and no settings window.

## Install

Download `simple-wacom-driver-setup.exe` from the [releases page](https://github.com/eriksimonic/simple-wacom-driver/releases) and run it. It needs no administrator rights for the install itself. It:

1. Copies the app to `%LOCALAPPDATA%\Programs\SimpleWacomDriver`.
2. Checks whether the tablet is bound to WinUSB. If not, it opens the bundled [Zadig](https://zadig.akeo.ie) (GPLv3, unmodified) and tells you what to click: pick "Wacom Tablet", then "Replace Driver". Zadig asks for administrator rights itself. This step cannot be automated: Zadig has no option to pick a device by USB ID, and Windows rejects an unsigned driver package, so the installer cannot do it alone.
3. Registers a scheduled task that starts the tool at every logon and starts it now.

Uninstall from Windows Settings > Apps. The tablet keeps the WinUSB driver after that. To go back to the Wacom driver: Device Manager, right-click the tablet, Uninstall device, tick "Delete the driver software", replug the tablet, and reinstall the Wacom driver.

Without the installer: bind the tablet with Zadig as above, then run `wacom.exe`, or `install.ps1` for start at logon.

In Photoshop, turn on Preferences > Tablet > Use Windows Ink and restart it.

## Tray icon

A tray icon shows the state. Right-click for the status line, "Open buttons.yaml" and Exit. If you cannot see the icon, check the hidden icons arrow in the taskbar.

## Modes

```
wacom.exe           pen mode: synthetic Windows Ink pen with pressure, tray icon, no console window
wacom.exe pen       the same, but keeps the terminal output
wacom.exe mouse     mouse mode: normal cursor, tip is left click, no pressure
wacom.exe print     print position and pressure, no tray icon
wacom.exe buttons   print the raw bytes of button changes, no tray icon
wacom.exe probe     guided recording for adding a new tablet, writes a report file
```

Started with no arguments from a terminal, the tool keeps printing there. Started from the logon task or a double-click, it has no console window. Use `pen` to see the foreground app names while you set up per-app curves.

If the tablet is unplugged, the tool waits and reconnects when it comes back.

## buttons.yaml

Read from the folder of `wacom.exe` at startup. Restart the tool after editing it. Upgrades keep your copy.

```yaml
tablet1: b              # tablet buttons, top to bottom
tablet2: e
tablet3: ctrl+alt+z
tablet4: ctrl+shift+z
pen_lower: barrel       # none, barrel (right click), eraser, or a shortcut
pen_upper: eraser
pressure_gamma: 1.2     # below 1 boosts light pressure, above 1 softens it
pressure_gamma.photoshop.exe: 1.0   # override per foreground app
```

The tool prints the exe name each time the foreground app changes.

## Why not a Windows service

A service runs in session 0 and cannot move the cursor or inject pen input into your desktop. The tool runs as a scheduled task at logon in your own session instead.

## Build

```
cargo build --release
```

To build the installer, install [Inno Setup](https://jrsoftware.org/isinfo.php), put `zadig-2.9.exe` from the [libwdi releases](https://github.com/pbatard/libwdi/releases/tag/v1.5.1) at `installer/zadig/zadig.exe`, and run `ISCC installer\setup.iss`.

Pushing a tag like `v0.1.0` makes GitHub Actions build the exe and the installer and publish a release.

## License

GPL-3.0-or-later, see [LICENSE](LICENSE). The installer bundles Zadig, also GPLv3, see `installer/ZADIG-NOTICE.txt`.
