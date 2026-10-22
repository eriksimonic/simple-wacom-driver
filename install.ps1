# Starts wacom.exe at every logon and starts it now. Run from the folder that holds wacom.exe.
$exe = Join-Path $PSScriptRoot 'wacom.exe'
if (-not (Test-Path $exe)) { throw "wacom.exe not found next to this script" }

$action = New-ScheduledTaskAction -Execute $exe -WorkingDirectory $PSScriptRoot
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
$settings = New-ScheduledTaskSettingsSet -Hidden -ExecutionTimeLimit ([TimeSpan]::Zero) `
    -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1)

Register-ScheduledTask -TaskName 'SimpleWacomDriver' -Action $action -Trigger $trigger -Settings $settings -Force | Out-Null
Start-ScheduledTask -TaskName 'SimpleWacomDriver'
Write-Host 'Installed. wacom.exe now starts at logon with a tray icon and no console window.'
