[CmdletBinding()]
param(
 [Parameter(Mandatory=$true)][int]$PanelPid,
 [Parameter(Mandatory=$true)][string]$ExpectedStartUtc,
 [Parameter(Mandatory=$true)][ValidatePattern('^C:\\ProgramData\\274bot-Test\\[^\\]+\\panel-play\.exe$')][string]$ExpectedBinary,
 [Parameter(Mandatory=$true)][ValidatePattern('^[0-9a-fA-F]{64}$')][string]$ExpectedBinarySha256,
 [Parameter(Mandatory=$true)][ValidateSet('BotTest')][string]$ExpectedUser,
 [Parameter(Mandatory=$true)][int]$ExpectedSessionId,
 [Parameter(Mandatory=$true)][int[]]$ExpectedRect,
 [Parameter(Mandatory=$true)][int[]]$GameImagePoint,
 [Parameter(Mandatory=$true)][string]$OutputDirectory,
 [Parameter(Mandatory=$true)][ValidatePattern('^[A-Za-z0-9_-]+$')][string]$Label,
 [Parameter(Mandatory=$true)][ValidateRange(1,900)][int]$DurationSeconds,
 [Parameter(Mandatory=$true)][switch]$CaptureEnabledVerified,
 [Parameter(Mandatory=$true)][switch]$SlotZeroFocusVerified
)
$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue'
$CadenceMilliseconds=1000;$PressMilliseconds=80

$out=[IO.Path]::GetFullPath($OutputDirectory)
$allowedRoots=@([IO.Path]::GetFullPath('C:\Users\BotTest\274bot-runs\visual-proof-'),[IO.Path]::GetFullPath('C:\Users\BotTest\274bot-runs\latency-diagnostic-'))
$receipt=Join-Path $out ($Label+'.input-stimulus.json')
$events=New-Object System.Collections.Generic.List[object]
$startedUtc=$null;$endedUtc=$null;$outcome='not-started';$terminalReason=$null;$failure=$null
$process=$null;$window=[IntPtr]::Zero;$initialRect=$null;$sessionId=$null;$binaryHash=$null;$finalBinaryHash=$null
$pressedKey=$null;$pressedAt=$null;$requested=0;$completed=0;$missed=0;$deferred=0
$stopwatch=[Diagnostics.Stopwatch]::StartNew();$receiptOwned=$false

function Write-Receipt {
 $windowValue=$null;if($window -ne [IntPtr]::Zero){$windowValue=$window.ToInt64()}
 $rectValue=$null;if($initialRect){$rectValue=@($initialRect.Left,$initialRect.Top,$initialRect.Right,$initialRect.Bottom)}
 [ordered]@{
  schema='native-panel-input-stimulus-v1';label=$Label;pid=$PanelPid;startUtc=$ExpectedStartUtc
  expectedUser=$ExpectedUser;sessionId=$sessionId;expectedSessionId=$ExpectedSessionId
  binary=$ExpectedBinary;binarySha256=$binaryHash;finalBinarySha256=$finalBinaryHash;expectedBinarySha256=$ExpectedBinarySha256
  windowHandle=$windowValue;expectedRect=$ExpectedRect;observedRect=$rectValue;gameImagePoint=$GameImagePoint
  captureEnabledVerified=[bool]$CaptureEnabledVerified;slotZeroFocusVerified=[bool]$SlotZeroFocusVerified
  cadenceMilliseconds=$CadenceMilliseconds;pressMilliseconds=$PressMilliseconds;durationSeconds=$DurationSeconds
  requestedPulses=$requested;completedPulses=$completed;missedSlots=$missed;deferredSlots=$deferred
  startedUtc=$startedUtc;endedUtc=$endedUtc;durationMilliseconds=$stopwatch.ElapsedMilliseconds
  events=$events.ToArray();outcome=$outcome;terminalReason=$terminalReason;failure=$failure
  performanceAcceptance=$false;inputCoveragePass=$false;hostAcknowledgement='not inferred from external stimulus'
 } | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $receipt -Encoding UTF8
}
function Assert-Target([bool]$CheckRect) {
 $process.Refresh()
 if($process.HasExited -or $process.ProcessName -ne 'panel-play' -or $process.Path -cne $ExpectedBinary){throw 'Target process identity changed'}
 if($process.StartTime.ToUniversalTime().ToString('o') -cne $ExpectedStartUtc -or $process.SessionId -ne $ExpectedSessionId){throw 'Target process start/session identity changed'}
 if($sessionId -ne (Get-Process -Id $PID).SessionId){throw 'Helper and target are not in the same local session'}
 if([PanelInputStimulusNative]::GetSystemMetrics(0x1000) -ne 0){throw 'Local console required'}
 if([PanelInputStimulusNative]::GetForegroundWindow() -ne $window){throw 'Target window is not foreground'}
 if([PanelInputStimulusNative]::IsIconic($window)){throw 'Target window is minimized'}
 $ownerPid=0;[void][PanelInputStimulusNative]::GetWindowThreadProcessId($window,[ref]$ownerPid)
 if($ownerPid -ne $PanelPid){throw 'Target window owner changed'}
 $rect=New-Object PanelInputStimulusNative+RECT
 if(-not [PanelInputStimulusNative]::GetWindowRect($window,[ref]$rect)){throw 'Target window bounds unavailable'}
 if($CheckRect -and ($rect.Left -ne $initialRect.Left -or $rect.Top -ne $initialRect.Top -or $rect.Right -ne $initialRect.Right -or $rect.Bottom -ne $initialRect.Bottom)){throw 'Target window rectangle changed'}
 return $rect
}

try {
 if($env:USERNAME -cne $ExpectedUser){throw 'Helper must run as the expected BotTest user'}
 if($ExpectedRect.Count -ne 4 -or $ExpectedRect[2] -le $ExpectedRect[0] -or $ExpectedRect[3] -le $ExpectedRect[1]){throw 'ExpectedRect must be four ordered physical coordinates'}
 if($GameImagePoint.Count -ne 2){throw 'GameImagePoint must contain exactly two explicit physical coordinates'}
 if(-not ($out.StartsWith($allowedRoots[0],[StringComparison]::OrdinalIgnoreCase) -or $out.StartsWith($allowedRoots[1],[StringComparison]::OrdinalIgnoreCase))){throw 'Output must be in a dedicated visual-proof or latency-diagnostic subtree'}
 if(-not (Test-Path -LiteralPath $out -PathType Container)){throw 'Prepared output directory missing'}
 if(Test-Path -LiteralPath $receipt -PathType Leaf){throw 'Stimulus label already exists; old outputs are never reused'}
 $old=Get-ChildItem -LiteralPath $out -Filter ($Label+'*') -File -Force -ErrorAction SilentlyContinue
 if($old.Count -ne 0){throw 'Stimulus label has existing output files'}
 $receiptOwned=$true
 if(-not [Environment]::Is64BitProcess){throw 'x64 PowerShell process required'}
 if($PSVersionTable.PSVersion.Major -ne 5 -or $PSVersionTable.PSVersion.Minor -ne 1 -or $PSEdition -ne 'Desktop'){throw 'Windows PowerShell 5.1 required'}
 if([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT){throw 'Windows PowerShell host required'}
 if(-not $CaptureEnabledVerified){throw 'Root must verify capture enabled from the actual panel UI before invoking this helper'}
 if(-not $SlotZeroFocusVerified){throw 'Root must verify slot-zero focus from the actual panel UI before invoking this helper'}

 Add-Type -TypeDefinition @'
 using System;
 using System.Runtime.InteropServices;
 public static class PanelInputStimulusNative {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left,Top,Right,Bottom; }
  [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT { public ushort wVk,wScan; public uint dwFlags,time; public UIntPtr dwExtraInfo; }
  [StructLayout(LayoutKind.Explicit, Size=40)] public struct INPUT { [FieldOffset(0)] public uint type; [FieldOffset(8)] public KEYBDINPUT keyboard; }
  public static INPUT CreateKeyboardInput(ushort vk, uint flags) { INPUT input=new INPUT(); input.type=1; KEYBDINPUT key=new KEYBDINPUT(); key.wVk=vk; key.wScan=0; key.dwFlags=flags; key.time=0; key.dwExtraInfo=UIntPtr.Zero; input.keyboard=key; return input; }
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
   [DllImport("user32.dll")] public static extern int GetWindowThreadProcessId(IntPtr h, out int pid);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X,Y; }
  [DllImport("user32.dll", SetLastError=true)] public static extern uint SendInput(uint n, INPUT[] inputs, int size);
 }
'@
 [void][PanelInputStimulusNative]::SetProcessDPIAware()
 if([PanelInputStimulusNative]::GetSystemMetrics(0x1000) -ne 0){throw 'Local console required'}
 if([Runtime.InteropServices.Marshal]::SizeOf((New-Object PanelInputStimulusNative+INPUT)) -ne 40){throw 'INPUT interop size is not 40 bytes'}
 $process=Get-Process -Id $PanelPid -ErrorAction Stop
 if($process.ProcessName -ne 'panel-play' -or $process.Path -cne $ExpectedBinary){throw 'Unexpected target executable'}
 if($process.StartTime.ToUniversalTime().ToString('o') -cne $ExpectedStartUtc){throw 'Target process start identity changed'}
 $sessionId=$process.SessionId
 if($sessionId -ne $ExpectedSessionId -or $sessionId -ne (Get-Process -Id $PID).SessionId){throw 'Target is not in the expected local session'}
 $binaryHash=(Get-FileHash -LiteralPath $ExpectedBinary -Algorithm SHA256).Hash.ToLower()
 if($binaryHash -cne $ExpectedBinarySha256.ToLower()){throw 'Frozen binary hash mismatch'}
 $process.Refresh();$window=$process.MainWindowHandle
 if($window -eq [IntPtr]::Zero){throw 'Target window is absent'}
 $initialRect=Assert-Target $false
 if($initialRect.Left -ne $ExpectedRect[0] -or $initialRect.Top -ne $ExpectedRect[1] -or $initialRect.Right -ne $ExpectedRect[2] -or $initialRect.Bottom -ne $ExpectedRect[3]){throw 'ExpectedRect does not match freshly inspected physical bounds'}
 if($GameImagePoint[0] -lt $initialRect.Left -or $GameImagePoint[0] -ge $initialRect.Right -or $GameImagePoint[1] -lt $initialRect.Top -or $GameImagePoint[1] -ge $initialRect.Bottom){throw 'GameImagePoint is outside the expected physical window rect'}
 if(-not [PanelInputStimulusNative]::SetCursorPos($GameImagePoint[0],$GameImagePoint[1])){throw 'SetCursorPos failed'}
 $point=New-Object PanelInputStimulusNative+POINT
 if(-not [PanelInputStimulusNative]::GetCursorPos([ref]$point) -or $point.X -ne $GameImagePoint[0] -or $point.Y -ne $GameImagePoint[1]){throw 'Cursor did not remain at the verified Game Image point'}
 $down=[PanelInputStimulusNative]::CreateKeyboardInput(0x25,0x0001)
 $up=[PanelInputStimulusNative]::CreateKeyboardInput(0x25,0x0003)
 if($down.type -ne 1 -or $up.type -ne 1 -or $down.keyboard.wVk -ne 0x25 -or $up.keyboard.wVk -ne 0x25 -or $down.keyboard.dwFlags -ne 1 -or $up.keyboard.dwFlags -ne 3){throw 'Left-arrow INPUT factory fields invalid'}
 $inputSize=[Runtime.InteropServices.Marshal]::SizeOf($down);$startedUtc=[DateTime]::UtcNow.ToString('o');$requested=$DurationSeconds;$receiptOwned=$true
 $stopwatch.Restart()
 $deadlineMilliseconds=($DurationSeconds*1000)+5000
 for($i=0;$i -lt $DurationSeconds;$i++) {
  $due=$i*$CadenceMilliseconds
  if($stopwatch.ElapsedMilliseconds -ge $deadlineMilliseconds){$missed+=($DurationSeconds-$i);$terminalReason='total-duration-limit';throw 'Total duration limit reached'}
  while($stopwatch.ElapsedMilliseconds -lt $due){
   $remaining=$due-$stopwatch.ElapsedMilliseconds
   if($stopwatch.ElapsedMilliseconds+$remaining -ge $deadlineMilliseconds){$missed+=($DurationSeconds-$i);$terminalReason='total-duration-limit';throw 'Total duration limit reached'}
   Start-Sleep -Milliseconds ([Math]::Min(25,[Math]::Max(1,$remaining)))
  }
  $late=$stopwatch.ElapsedMilliseconds-$due
  if($late -ge $CadenceMilliseconds){$deferred++;$missed+=($DurationSeconds-$i);$terminalReason='cadence-delayed-no-catch-up';throw 'Cadence delayed; refusing catch-up keystrokes'}
  $pulseRect=Assert-Target $true
  if($GameImagePoint[0] -lt $pulseRect.Left -or $GameImagePoint[0] -ge $pulseRect.Right -or $GameImagePoint[1] -lt $pulseRect.Top -or $GameImagePoint[1] -ge $pulseRect.Bottom){throw 'GameImagePoint is outside the current physical window rect'}
  if(-not [PanelInputStimulusNative]::SetCursorPos($GameImagePoint[0],$GameImagePoint[1])){throw 'SetCursorPos failed before pulse'}
  $pulsePoint=New-Object PanelInputStimulusNative+POINT
  if(-not [PanelInputStimulusNative]::GetCursorPos([ref]$pulsePoint) -or $pulsePoint.X -ne $GameImagePoint[0] -or $pulsePoint.Y -ne $GameImagePoint[1]){throw 'Cursor did not remain at the verified Game Image point before pulse'}
  $vk=if(($i % 2) -eq 0){0x25}else{0x27};$downFlags=0x0001;$upFlags=0x0003
  $down=[PanelInputStimulusNative]::CreateKeyboardInput([ushort]$vk,$downFlags);$up=[PanelInputStimulusNative]::CreateKeyboardInput([ushort]$vk,$upFlags)
  if($down.keyboard.wVk -ne $vk -or $up.keyboard.wVk -ne $vk -or $down.keyboard.dwFlags -ne $downFlags -or $up.keyboard.dwFlags -ne $upFlags){throw 'Arrow INPUT factory fields invalid'}
  $direction=if($vk -eq 0x25){'Left'}else{'Right'}
  $event=[ordered]@{index=$i+1;direction=$direction;virtualKey=$vk;downFlags=$downFlags;upFlags=$upFlags;dueMilliseconds=$due;latenessMilliseconds=$late;actualDownElapsedMilliseconds=$null;actualDownLatenessMilliseconds=$null;requestedUtc=[DateTime]::UtcNow.ToString('o');downUtc=$null;upUtc=$null;downSendInputResult=$null;upSendInputResult=$null;upSendInputException=$null;releaseAttempted=$false;releaseSucceeded=$false}
  $downSucceeded=$false
  try {
   Assert-Target $true | Out-Null
   $event.actualDownElapsedMilliseconds=$stopwatch.ElapsedMilliseconds;$event.actualDownLatenessMilliseconds=$event.actualDownElapsedMilliseconds-$due
   if($event.actualDownElapsedMilliseconds -ge $deadlineMilliseconds){$terminalReason='total-duration-limit';throw 'Total duration limit reached after target guards'}
   if($event.actualDownLatenessMilliseconds -ge $CadenceMilliseconds){$deferred++;$terminalReason='cadence-delayed-no-catch-up';throw 'Cadence delayed after target guards; refusing catch-up keystrokes'}
   $event.downUtc=[DateTime]::UtcNow.ToString('o');$event.downSendInputResult=[PanelInputStimulusNative]::SendInput(1,@($down),$inputSize)
   if($event.downSendInputResult -ne 1){throw 'Arrow key-down SendInput failed'}
   $downSucceeded=$true;$pressedKey=$vk;$pressedAt=$event.downUtc
   Start-Sleep -Milliseconds $PressMilliseconds
  } finally {
   if($downSucceeded){$event.releaseAttempted=$true;$event.upUtc=[DateTime]::UtcNow.ToString('o');try{$event.upSendInputResult=[PanelInputStimulusNative]::SendInput(1,@($up),$inputSize)}catch{$event.upSendInputException=$_.Exception.Message};$event.releaseSucceeded=($event.upSendInputResult -eq 1);if($event.releaseSucceeded){$pressedKey=$null;$pressedAt=$null}}
   $events.Add($event);if($downSucceeded -and $event.releaseSucceeded){$completed++}
  }
  if($downSucceeded -and -not $event.releaseSucceeded){$terminalReason='key-release-failed';throw 'Arrow key-up SendInput failed'}
 }
 if($stopwatch.ElapsedMilliseconds -ge $deadlineMilliseconds){$terminalReason='total-duration-limit';throw 'Total duration limit reached after final release'}
 $finalBinaryHash=(Get-FileHash -LiteralPath $ExpectedBinary -Algorithm SHA256).Hash.ToLower()
 if($finalBinaryHash -cne $ExpectedBinarySha256.ToLower()){throw 'Frozen binary hash changed at final verification'}
 if($stopwatch.ElapsedMilliseconds -ge $deadlineMilliseconds){$terminalReason='total-duration-limit';throw 'Total duration limit reached after final verification'}
 $outcome='completed';$terminalReason='all-predeclared-pulses-completed'
} catch {
 $outcome='failed';if(-not $terminalReason){$terminalReason='guard-or-input-failure'};$failure=$_.Exception.Message
 if($requested -gt $completed){$missed=[Math]::Max($missed,$requested-$completed)}
} finally {
 if($null -ne $pressedKey){try{$release=[PanelInputStimulusNative]::CreateKeyboardInput([ushort]$pressedKey,0x0003);$releaseResult=[PanelInputStimulusNative]::SendInput(1,@($release),[Runtime.InteropServices.Marshal]::SizeOf($release));if($releaseResult -ne 1){$failure=($failure+'; finally key release failed');$outcome='failed'}else{$pressedKey=$null;$pressedAt=$null}}catch{$failure=($failure+'; finally key release exception: '+$_.Exception.Message);$outcome='failed'}}
 $endedUtc=[DateTime]::UtcNow.ToString('o');if($receiptOwned){Write-Receipt}
}
if($receiptOwned){Get-Content -LiteralPath $receipt -Raw}
if($outcome -ne 'completed'){throw ('Panel input stimulus '+$outcome+': '+$terminalReason)}
