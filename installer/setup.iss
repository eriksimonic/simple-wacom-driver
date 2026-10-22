#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{6F0C3D52-7B1E-4C58-9A57-3E1D6B7A2F10}
AppName=Simple Wacom driver
AppVersion={#AppVersion}
AppPublisher=Erik Simonic
AppPublisherURL=https://github.com/eriksimonic/simple-wacom-driver
DefaultDirName={localappdata}\Programs\SimpleWacomDriver
DefaultGroupName=Simple Wacom driver
PrivilegesRequired=lowest
LicenseFile=..\LICENSE
OutputDir=Output
OutputBaseFilename=simple-wacom-driver-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayName=Simple Wacom driver
DisableProgramGroupPage=yes

[Files]
Source: "..\target\release\wacom.exe"; DestDir: "{app}"; Flags: ignoreversion
; keep the user's settings on upgrade
Source: "..\buttons.yaml"; DestDir: "{app}"; Flags: onlyifdoesntexist uninsneveruninstall
Source: "..\install.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\uninstall.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\check-winusb.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "ZADIG-NOTICE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "zadig\zadig.exe"; DestDir: "{app}\zadig"; Flags: ignoreversion
Source: "zadig\zadig.ini"; DestDir: "{app}\zadig"; Flags: ignoreversion

[Icons]
Name: "{group}\Edit buttons.yaml"; Filename: "notepad.exe"; Parameters: """{app}\buttons.yaml"""
Name: "{group}\Bind the tablet to WinUSB (Zadig)"; Filename: "{app}\zadig\zadig.exe"; WorkingDir: "{app}\zadig"
Name: "{group}\Start the driver"; Filename: "powershell.exe"; Parameters: "-NoProfile -WindowStyle Hidden -Command ""Start-ScheduledTask -TaskName SimpleWacomDriver"""

[Run]
Filename: "powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\install.ps1"""; Flags: runhidden; StatusMsg: "Registering the logon task..."

[UninstallRun]
Filename: "powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\uninstall.ps1"""; Flags: runhidden; RunOnceId: "RemoveTask"

[Code]
// 0 = bound to WinUSB, 1 = not bound, 2 = tablet not plugged in
function WinUsbState(): Integer;
var
  Code: Integer;
begin
  if not Exec('powershell.exe', '-NoProfile -ExecutionPolicy Bypass -File "' + ExpandConstant('{app}\check-winusb.ps1') + '"',
              '', SW_HIDE, ewWaitUntilTerminated, Code) then
    Code := 1;
  Result := Code;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Code: Integer;
begin
  if CurStep = ssInstall then
  begin
    // an old copy holds the exe open
    Exec('taskkill.exe', '/F /IM wacom.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
  end;

  if CurStep = ssPostInstall then
  begin
    case WinUsbState() of
      2: MsgBox('The tablet is not plugged in, so its driver cannot be checked.' + #13#10 +
                'Plug it in and run "Bind the tablet to WinUSB (Zadig)" from the Start menu.', mbInformation, MB_OK);
      1:
        begin
          MsgBox('The tablet needs the WinUSB driver. Zadig opens next. In Zadig:' + #13#10 + #13#10 +
                 '1. Pick "Wacom Tablet" (USB ID 056A 0378) in the list.' + #13#10 +
                 '2. Check that the driver box shows WinUSB.' + #13#10 +
                 '3. Click "Replace Driver" (or "Install Driver").' + #13#10 + #13#10 +
                 'The cursor stops following the pen until the new driver is in place.',
                 mbInformation, MB_OK);
          ShellExec('runas', ExpandConstant('{app}\zadig\zadig.exe'), '', ExpandConstant('{app}\zadig'),
                    SW_SHOWNORMAL, ewWaitUntilTerminated, Code);
          if WinUsbState() <> 0 then
            MsgBox('The tablet is still not on WinUSB. Run "Bind the tablet to WinUSB (Zadig)" from the Start menu.',
                   mbError, MB_OK);
        end;
    end;
  end;
end;
