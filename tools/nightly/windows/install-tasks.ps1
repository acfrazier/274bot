# Windows nightly host: register (or with -Remove, unregister) the two
# scheduled tasks for the current user (BotTest). No elevation: both tasks run
# with the Limited run level, only while the user is logged on (Interactive),
# and store no password.
#   274bot-nightly          daily at -At (default 23:00, host time = America/New_York):
#                           the whole night in the console session.
#   274bot-nightly-catchup  at this user's logon: if the last scheduled night never
#                           started (no session at 23:00), record it as UNAVAILABLE.
# Idempotent: re-running replaces both tasks with the same definition.
param(
    [string]$At = '23:00',
    [switch]$Remove
)
$ErrorActionPreference = 'Stop'
$Root = 'C:\274bot-nightly'
$Launcher = Join-Path $Root 'runner\tools\nightly\windows\nightly-launch.vbs'
$User = "$env:COMPUTERNAME\$env:USERNAME"
foreach ($name in '274bot-nightly', '274bot-nightly-catchup') {
    if (Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue) {
        Unregister-ScheduledTask -TaskName $name -Confirm:$false
    }
}
if ($Remove) { 'removed 274bot-nightly and 274bot-nightly-catchup'; return }
if (-not (Test-Path -LiteralPath $Launcher)) { throw "launcher missing: $Launcher" }
$Principal = New-ScheduledTaskPrincipal -UserId $User -LogonType Interactive -RunLevel Limited
$Night = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
    -MultipleInstances IgnoreNew -ExecutionTimeLimit (New-TimeSpan -Hours 11)
$Catch = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
    -MultipleInstances IgnoreNew -ExecutionTimeLimit (New-TimeSpan -Minutes 30)
# A start boundary without a UTC offset keeps 23:00 local through DST changes
# (with an offset, Task Scheduler would fire at a fixed UTC time).
$Daily = New-ScheduledTaskTrigger -Daily -At $At
$Daily.StartBoundary = (Get-Date $At).ToString('yyyy-MM-ddTHH:mm:ss')
Register-ScheduledTask -TaskName '274bot-nightly' -Principal $Principal -Settings $Night `
    -Trigger $Daily `
    -Action (New-ScheduledTaskAction -Execute 'wscript.exe' -Argument "`"$Launcher`" night" -WorkingDirectory $Root) `
    -Description '274bot Windows nightly: CI lanes, panel-play and tui-play cells (tools/nightly/README.md)' | Out-Null
Register-ScheduledTask -TaskName '274bot-nightly-catchup' -Principal $Principal -Settings $Catch `
    -Trigger (New-ScheduledTaskTrigger -AtLogOn -User $User) `
    -Action (New-ScheduledTaskAction -Execute 'wscript.exe' -Argument "`"$Launcher`" catch-up" -WorkingDirectory $Root) `
    -Description '274bot Windows nightly: record a missed night as UNAVAILABLE at logon' | Out-Null
Get-ScheduledTask -TaskName '274bot-nightly', '274bot-nightly-catchup' |
    Select-Object TaskName, State, @{ n = 'User'; e = { $_.Principal.UserId } }, @{ n = 'Logon'; e = { $_.Principal.LogonType } },
        @{ n = 'RunLevel'; e = { $_.Principal.RunLevel } }, @{ n = 'Next'; e = { (Get-ScheduledTaskInfo $_).NextRunTime } } |
    Format-Table -AutoSize | Out-String -Width 200
