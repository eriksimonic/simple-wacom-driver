# simple-wacom-driver

Small Windows tool for a Wacom Intuos (CTL-6100WL, USB `056A:0378`). It reads the tablet over WinUSB and gives pen pressure to Photoshop and Lightroom Classic through Windows Ink. The Wacom driver is not needed.

Only the CTL-6100WL is supported. Other models need their own VID, PID and report layout in `src/main.rs`.

## Setup

1. Uninstall the Wacom driver, or accept that it stops working for this tablet.
2. Run [Zadig](https://zadig.akeo.ie) as administrator. Options > List All Devices, pick "Wacom Tablet" (`056A 0378`), set the driver to WinUSB, click Replace Driver.
3. Run `wacom.exe`. The tablet now moves the cursor and sends pressure.
4. In Photoshop, turn on Preferences > Tablet > Use Windows Ink and restart it.

Zadig cannot be skipped. A program can only read the tablet directly when the WinUSB driver is bound to it.

To go back to the Wacom driver: Device Manager, right-click the tablet, Uninstall device, tick "Delete the driver software", replug the tablet.

## Modes

```
wacom.exe           pen mode: synthetic Windows Ink pen with pressure
wacom.exe mouse     mouse mode: normal cursor, tip is left click, no pressure
wacom.exe print     print position and pressure
wacom.exe buttons   print the raw bytes of button changes
```

If the tablet is unplugged, the tool waits and reconnects when it comes back.

## buttons.yaml

Read from the folder of `wacom.exe` at startup. Restart the tool after editing it.

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

## Start at logon

```
powershell -ExecutionPolicy Bypass -File install.ps1
powershell -ExecutionPolicy Bypass -File uninstall.ps1
```

This is a scheduled task that runs at logon in your session, not a Windows service. A service runs in session 0 and cannot move the cursor or inject pen input into your desktop.

## Build

```
cargo build --release
```

Pushing a tag like `v0.1.0` makes GitHub Actions build the exe and publish a release zip.
