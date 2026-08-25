[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ClientBinary,
    [Parameter(Mandatory = $true)][string]$StatusBinary,
    [string]$Config = ""
)

$ErrorActionPreference = "Stop"
$installDir = Join-Path $env:LOCALAPPDATA "RemoteInputBridge"
$clientTarget = Join-Path $installDir "remote-input-client.exe"
$statusTarget = Join-Path $installDir "remote-input-status.exe"
$configTarget = Join-Path $installDir "config.toml"

New-Item -ItemType Directory -Path $installDir -Force | Out-Null
Copy-Item -LiteralPath (Resolve-Path -LiteralPath $ClientBinary) -Destination $clientTarget -Force
Copy-Item -LiteralPath (Resolve-Path -LiteralPath $StatusBinary) -Destination $statusTarget -Force
if ($Config) {
    Copy-Item -LiteralPath (Resolve-Path -LiteralPath $Config) -Destination $configTarget -Force
}
elseif (-not (Test-Path -LiteralPath $configTarget)) {
    throw "No configuration exists. Pass -Config with a reviewed config.toml."
}

$action = New-ScheduledTaskAction -Execute $clientTarget -Argument "--config `"$configTarget`""
$trigger = New-ScheduledTaskTrigger -AtLogOn -User "$env:USERDOMAIN\$env:USERNAME"
$principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType Interactive -RunLevel Limited
$settings = New-ScheduledTaskSettingsSet -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew
Register-ScheduledTask -TaskName "Remote Input Bridge" -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null

Write-Host "Installed Remote Input Bridge in $installDir"
Write-Host "Review $configTarget, then start it with: Start-ScheduledTask -TaskName 'Remote Input Bridge'"

