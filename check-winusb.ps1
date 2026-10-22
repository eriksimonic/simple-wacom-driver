# Exit code 0: the tablet is bound to WinUSB. 1: it is not. 2: the tablet is not plugged in.
$device = Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue |
    Where-Object { $_.InstanceId -like 'USB\VID_056A&PID_0378*' } | Select-Object -First 1
if (-not $device) { exit 2 }
$service = (Get-PnpDeviceProperty -InstanceId $device.InstanceId -KeyName 'DEVPKEY_Device_Service').Data
if ($service -eq 'WinUSB') { exit 0 } else { exit 1 }
