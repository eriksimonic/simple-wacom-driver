# Stops wacom.exe and removes the logon task.
Stop-ScheduledTask -TaskName 'SimpleWacomDriver' -ErrorAction SilentlyContinue
Unregister-ScheduledTask -TaskName 'SimpleWacomDriver' -Confirm:$false -ErrorAction SilentlyContinue
Stop-Process -Name wacom -Force -ErrorAction SilentlyContinue
Write-Host 'Removed.'
