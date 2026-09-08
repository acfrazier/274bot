param(
 [Parameter(Mandatory=$true)][int]$PanelPid,
 [Parameter(Mandatory=$true)][string]$ExpectedStartUtc,
 [Parameter(Mandatory=$true)][string]$OutputDirectory,
 [Parameter(Mandatory=$true)][ValidatePattern('^[A-Za-z0-9_-]+$')][string]$Label,
 [Parameter(Mandatory=$true)][int]$InputX,
 [Parameter(Mandatory=$true)][int]$InputY,
 [Parameter(Mandatory=$true)][int[]]$ExpectedRect
)
$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue'

$expectedBinary='C:\ProgramData\274bot-Test\tile-boxed-fb3589a\panel-play.exe'
$expectedBinaryHash='a9b581bab780d2868035941d799d416f0ef9ec68d5eb5a4fe0a62fa29670667f'
$helperPath=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'capture-panel-burst.ps1'))
$expectedHelperHash='c09639cf3f94dcdadb276eb9a4b0256c81e4b988207b8d748d636c9e9b7ba181'
$out=[IO.Path]::GetFullPath($OutputDirectory)
$prefix=[IO.Path]::GetFullPath('C:\Users\BotTest\274bot-runs\visual-proof-')
$controllerReceipt=Join-Path $out ($Label+'.controller.json')
$stdoutPath=Join-Path $out ($Label+'.helper.stdout.txt')
$stderrPath=Join-Path $out ($Label+'.helper.stderr.txt')
$child=$null;$childPid=$null;$childStartUtc=$null;$childHandle=$null;$receiptOwned=$false
$controllerStartUtc=[DateTime]::UtcNow.ToString('o')
$inputBeforeUtc=$null;$inputDownUtc=$null;$inputUpUtc=$null
$readyUtc=$null;$firstFrame=$null;$outcome='not-started';$failure=$null;$childExitCode=$null
$targetWindow=[IntPtr]::Zero;$initialRect=$null;$binaryHash=$null;$helperHash=$null;$sessionId=$null
$inputIdentity=$null;$clickAttempted=$false;$downSendInputResult=$null;$upSendInputResult=$null;$releaseFailure=$null
$burst=$null;$burstOutcome=$null;$burstIdentityMatch=$false;$stopwatch=[Diagnostics.Stopwatch]::StartNew()
$childAliveAtInput=$false;$receiptWritten=$false

function Quote-Argument([string]$Value) {
 return '"'+($Value -replace '(\\*)"','$1$1\"' -replace '(\\+)$','$1$1')+'"'
}
function Read-FirstFrame {
 $files=@(Get-ChildItem -LiteralPath $out -Filter ($Label+'-frame-0001-*.json') -File -Force -ErrorAction SilentlyContinue)
 if($files.Count -ne 1){return $null}
 return (Get-Content -LiteralPath $files[0].FullName -Raw | ConvertFrom-Json)
}
function Verify-Target([bool]$CheckRect) {
 $script:process.Refresh()
 if($script:process.HasExited -or $script:process.ProcessName -ne 'panel-play' -or $script:process.Path -ne $expectedBinary){throw 'Target process identity changed'}
 if($script:process.StartTime.ToUniversalTime().ToString('o') -ne $ExpectedStartUtc -or $script:process.SessionId -ne $sessionId){throw 'Target process start/session identity changed'}
 if([PanelRebuildCaptureNative]::GetForegroundWindow() -ne $targetWindow){throw 'Target window is not foreground'}
 if([PanelRebuildCaptureNative]::IsIconic($targetWindow)){throw 'Target window is minimized'}
 $ownerPid=0;[void][PanelRebuildCaptureNative]::GetWindowThreadProcessId($targetWindow,[ref]$ownerPid)
 if($ownerPid -ne $PanelPid){throw 'Target window owner changed'}
 $rect=New-Object PanelRebuildCaptureNative+RECT
 if(-not [PanelRebuildCaptureNative]::GetWindowRect($targetWindow,[ref]$rect)){throw 'Target window bounds unavailable'}
 if($CheckRect -and ($rect.Left -ne $initialRect.Left -or $rect.Top -ne $initialRect.Top -or $rect.Right -ne $initialRect.Right -or $rect.Bottom -ne $initialRect.Bottom)){throw 'Target window rectangle changed'}
 return $rect
}

try {
 if($env:USERNAME -ne 'BotTest'){throw 'Controller must run in the BotTest interactive session'}
 if($ExpectedRect.Count -ne 4){throw 'ExpectedRect must contain exactly four explicit integers'}
 if($InputX -lt 0 -or $InputY -lt 0){throw 'Input coordinates must be non-negative window-relative integers'}
 if(-not (Test-Path -LiteralPath $out -PathType Container)){throw 'Prepared output directory missing'}
 if(-not $out.StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase)){throw 'Output must be in a dedicated visual-proof directory'}
 if((Test-Path -LiteralPath $controllerReceipt -PathType Leaf)){throw 'Controller label already exists'}
 if((Test-Path -LiteralPath $stdoutPath -PathType Leaf) -or (Test-Path -LiteralPath $stderrPath -PathType Leaf)){throw 'Controller output files already exist'}
 if(-not (Test-Path -LiteralPath $helperPath -PathType Leaf)){throw 'Existing burst helper missing'}
 $helperHash=(Get-FileHash -LiteralPath $helperPath -Algorithm SHA256).Hash.ToLower()
 if($helperHash -ne $expectedHelperHash){throw 'Existing burst helper hash mismatch'}
 $receiptOwned=$true

 Add-Type -TypeDefinition @'
 using System;
 using System.Runtime.InteropServices;
 public static class PanelRebuildCaptureNative {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left,Top,Right,Bottom; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X,Y; }
  [StructLayout(LayoutKind.Sequential)] public struct MOUSEINPUT { public int dx,dy; public uint mouseData, dwFlags, time; public UIntPtr dwExtraInfo; }
  [StructLayout(LayoutKind.Explicit, Size=40)] public struct INPUT { [FieldOffset(0)] public uint type; [FieldOffset(8)] public MOUSEINPUT mouse; }
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern int GetWindowThreadProcessId(IntPtr h, out int pid);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll", SetLastError=true)] public static extern uint SendInput(uint n, INPUT[] inputs, int size);
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
 }
'@
 [void][PanelRebuildCaptureNative]::SetProcessDPIAware()
 if([PanelRebuildCaptureNative]::GetSystemMetrics(0x1000) -ne 0){throw 'Local console required'}
 $script:process=Get-Process -Id $PanelPid -ErrorAction Stop
 if($process.ProcessName -ne 'panel-play' -or $process.Path -ne $expectedBinary){throw 'Unexpected target executable'}
 if($process.StartTime.ToUniversalTime().ToString('o') -ne $ExpectedStartUtc){throw 'Target process start identity changed'}
 $sessionId=$process.SessionId
 if($sessionId -ne (Get-Process -Id $PID).SessionId){throw 'Target belongs to another session'}
 $binaryHash=(Get-FileHash -LiteralPath $expectedBinary -Algorithm SHA256).Hash.ToLower()
 if($binaryHash -ne $expectedBinaryHash){throw 'Frozen binary hash mismatch'}
 $process.Refresh();$targetWindow=$process.MainWindowHandle
 if($targetWindow -eq [IntPtr]::Zero -or [PanelRebuildCaptureNative]::IsIconic($targetWindow)){throw 'Target window absent or minimized'}
 $initialRect=Verify-Target $false
 if($initialRect.Left -ne $ExpectedRect[0] -or $initialRect.Top -ne $ExpectedRect[1] -or $initialRect.Right -ne $ExpectedRect[2] -or $initialRect.Bottom -ne $ExpectedRect[3]){throw 'ExpectedRect does not match the freshly inspected target window'}
 $width=$initialRect.Right-$initialRect.Left;$height=$initialRect.Bottom-$initialRect.Top
 if($width -le 0 -or $height -le 0 -or $width -gt 8192 -or $height -gt 8192){throw 'Invalid target window bounds'}
 if($InputX -ge $width -or $InputY -ge $height){throw 'Input coordinates are outside the target window bounds'}

 $psi=New-Object Diagnostics.ProcessStartInfo
 $psi.FileName=Join-Path $env:WINDIR 'System32\WindowsPowerShell\v1.0\powershell.exe'
 $psi.UseShellExecute=$false;$psi.CreateNoWindow=$true;$psi.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
 $psi.RedirectStandardOutput=$true;$psi.RedirectStandardError=$true
 $args=@('-NoLogo','-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',$helperPath,'-PanelPid',[string]$PanelPid,'-ExpectedStartUtc',$ExpectedStartUtc,'-OutputDirectory',$out,'-Label',$Label,'-DurationSeconds','30','-MaxFrames','300','-DelayMilliseconds','100')
 $psi.Arguments=(($args | ForEach-Object {Quote-Argument ([string]$_)}) -join ' ')
 $child=New-Object Diagnostics.Process;$child.StartInfo=$psi
 $childStartUtc=[DateTime]::UtcNow.ToString('o')
 if(-not $child.Start()){throw 'Could not start the existing capture helper'}
 $childPid=$child.Id
 $childStartUtc=$child.StartTime.ToUniversalTime().ToString('o')
 $childHandle=$child.Handle
 $child.Refresh()
 if($child.SessionId -ne $sessionId){throw 'Capture helper belongs to another Windows session'}
 $stdoutTask=$child.StandardOutput.ReadToEndAsync();$stderrTask=$child.StandardError.ReadToEndAsync()
 $readyPath=Join-Path $out ($Label+'.ready')
 while($stopwatch.Elapsed.TotalSeconds -lt 60) {
  if($child.HasExited){break}
  if(Test-Path -LiteralPath $readyPath -PathType Leaf){
   $readyText=Get-Content -LiteralPath $readyPath -Raw
   if($readyText -notmatch '^readyUtc=(?<utc>[^\r\n]+)\s*$'){throw 'Ready marker format invalid'}
   $readyUtc=$Matches.utc
   $firstFrame=Read-FirstFrame
   if($null -ne $firstFrame){break}
  }
  Start-Sleep -Milliseconds 50
 }
 if($null -eq $firstFrame){throw 'Bounded wait ended without a unique fresh ready marker and first frame receipt'}
 if($child.HasExited){throw 'Capture helper exited before synchronized input'}
 $firstStart=[DateTime]::Parse($firstFrame.captureStartedUtc).ToUniversalTime()
 if($firstStart -lt [DateTime]::Parse($controllerStartUtc).ToUniversalTime()){throw 'First frame predates controller start'}
 $readyTime=[DateTime]::Parse($readyUtc).ToUniversalTime()
 $firstEnd=[DateTime]::Parse($firstFrame.captureEndedUtc).ToUniversalTime()
 if($readyTime -lt $firstStart -or $readyTime -lt $firstEnd){throw 'Ready timestamp is inconsistent with first frame'}
 if([string]$firstFrame.label -ne $Label -or [int]$firstFrame.frame -ne 1 -or [int]$firstFrame.pid -ne $PanelPid -or [string]$firstFrame.startUtc -ne $ExpectedStartUtc -or [string]$firstFrame.binary -ne $expectedBinary -or [string]$firstFrame.binarySha256 -ne $binaryHash -or [Int64]$firstFrame.windowHandle -ne $targetWindow.ToInt64()){throw 'First frame identity does not match expected target'}
 $frameRect=@($firstFrame.rect | ForEach-Object {[int]$_})
 if($frameRect.Count -ne 4 -or $frameRect[0] -ne $ExpectedRect[0] -or $frameRect[1] -ne $ExpectedRect[1] -or $frameRect[2] -ne $ExpectedRect[2] -or $frameRect[3] -ne $ExpectedRect[3]){throw 'First frame rectangle does not match expected target'}
 if(-not (Test-Path -LiteralPath ([string]$firstFrame.png) -PathType Leaf)){throw 'First frame PNG receipt is missing'}
 if((Get-FileHash -LiteralPath ([string]$firstFrame.png) -Algorithm SHA256).Hash.ToLower() -ne ([string]$firstFrame.pngSha256).ToLower()){throw 'First frame PNG hash mismatch'}

 $null=Verify-Target $true
 $inputBeforeUtc=[DateTime]::UtcNow.ToString('o')
 $screenX=$initialRect.Left+$InputX;$screenY=$initialRect.Top+$InputY
 if(-not [PanelRebuildCaptureNative]::SetCursorPos($screenX,$screenY)){throw 'SetCursorPos failed'}
 $point=New-Object PanelRebuildCaptureNative+POINT
 if(-not [PanelRebuildCaptureNative]::GetCursorPos([ref]$point) -or $point.X -ne $screenX -or $point.Y -ne $screenY){throw 'Cursor identity changed before click'}
 $inputIdentity=[ordered]@{windowRelativeX=$InputX;windowRelativeY=$InputY;screenX=$screenX;screenY=$screenY;windowHandle=$targetWindow.ToInt64();pid=$PanelPid;startUtc=$ExpectedStartUtc;rect=$ExpectedRect}
 $null=Verify-Target $true
 if($child.HasExited){throw 'Capture helper exited before input'}
 $childAliveAtInput=$true
 $down=New-Object PanelRebuildCaptureNative+INPUT;$down.type=0;$down.mouse.dwFlags=0x0002
 $up=New-Object PanelRebuildCaptureNative+INPUT;$up.type=0;$up.mouse.dwFlags=0x0004
 $inputSize=[Runtime.InteropServices.Marshal]::SizeOf($down)
 $clickAttempted=$true
 try {
  $inputDownUtc=[DateTime]::UtcNow.ToString('o')
  $downSendInputResult=[PanelRebuildCaptureNative]::SendInput(1,@($down),$inputSize)
  if($downSendInputResult -ne 1){throw 'Mouse-down SendInput failed'}
  Start-Sleep -Milliseconds 80
 } finally {
  $inputUpUtc=[DateTime]::UtcNow.ToString('o')
  $upSendInputResult=[PanelRebuildCaptureNative]::SendInput(1,@($up),$inputSize)
  if($upSendInputResult -ne 1){$releaseFailure='Mouse-up SendInput failed'}
 }
 if($null -ne $releaseFailure){throw $releaseFailure}
 $null=Verify-Target $true
 while(-not $child.HasExited -and $stopwatch.Elapsed.TotalSeconds -lt 60){Start-Sleep -Milliseconds 100}
 if(-not $child.HasExited){throw 'Capture helper exceeded the 60 second controller bound'}
 $childExitCode=$child.ExitCode
 $outcome='completed'
 $burstPath=Join-Path $out ($Label+'.burst.json')
 if(-not (Test-Path -LiteralPath $burstPath -PathType Leaf)){throw 'Capture helper exited without burst receipt'}
 $burst=Get-Content -LiteralPath $burstPath -Raw | ConvertFrom-Json
 $burstOutcome=[string]$burst.outcome
 if($burstOutcome -ne 'completed'){throw ('Capture burst terminal outcome was '+$burstOutcome)}
 $burstStart=[DateTime]::Parse($burst.captureStartedUtc).ToUniversalTime()
 $burstEnd=[DateTime]::Parse($burst.captureEndedUtc).ToUniversalTime()
 $downTime=[DateTime]::Parse($inputDownUtc).ToUniversalTime();$upTime=[DateTime]::Parse($inputUpUtc).ToUniversalTime()
 if($downTime -lt $burstStart -or $upTime -gt $burstEnd -or $upTime -lt $downTime){throw 'Complete click interval was not inside the accepted burst'}
 if([string]$burst.label -ne $Label -or [int]$burst.pid -ne $PanelPid -or [string]$burst.startUtc -ne $ExpectedStartUtc -or [string]$burst.binary -ne $expectedBinary -or [string]$burst.binarySha256 -ne $binaryHash -or [Int64]$burst.windowHandle -ne $targetWindow.ToInt64()){throw 'Burst identity does not match expected target'}
 $burstRect=@($burst.rect | ForEach-Object {[int]$_})
 if($burstRect.Count -ne 4 -or $burstRect[0] -ne $ExpectedRect[0] -or $burstRect[1] -ne $ExpectedRect[1] -or $burstRect[2] -ne $ExpectedRect[2] -or $burstRect[3] -ne $ExpectedRect[3]){throw 'Burst rectangle does not match expected target'}
 $burstIdentityMatch=$true
} catch {
 $outcome='failed';$failure=$_.Exception.Message
} finally {
 if($null -ne $child -and $child.HasExited -eq $false){
  try {
   $child.Refresh()
   if($child.Id -eq $childPid -and $child.StartTime.ToUniversalTime().ToString('o') -eq $childStartUtc){
    $child.Kill();$remainingMs=[Math]::Max(0,60000-[int]$stopwatch.ElapsedMilliseconds)
    if(-not $child.WaitForExit([Math]::Min(5000,$remainingMs))){throw 'Capture helper did not stop within cleanup bound'}
   } else {throw 'Capture helper identity changed; refusing cleanup'}
  } catch {$failure=($failure+'; cleanup: '+$_.Exception.Message)}
 }
 if($null -ne $child -and $child.HasExited){try{$childExitCode=$child.ExitCode}catch{}}
 if($null -ne $stdoutTask){try{[IO.File]::WriteAllText($stdoutPath,$stdoutTask.GetAwaiter().GetResult())}catch{$failure=($failure+'; stdout: '+$_.Exception.Message)}}
 if($null -ne $stderrTask){try{[IO.File]::WriteAllText($stderrPath,$stderrTask.GetAwaiter().GetResult())}catch{$failure=($failure+'; stderr: '+$_.Exception.Message)}}
 $controllerEndUtc=[DateTime]::UtcNow.ToString('o')
 if($receiptOwned){[ordered]@{schema='native-synchronized-rebuild-controller-v1';label=$Label;controllerStartUtc=$controllerStartUtc;controllerEndUtc=$controllerEndUtc;controllerDurationMilliseconds=$stopwatch.ElapsedMilliseconds;panelPid=$PanelPid;expectedStartUtc=$ExpectedStartUtc;sessionId=$sessionId;binary=$expectedBinary;binarySha256=$binaryHash;helperPath=$helperPath;helperSha256=$helperHash;helperPid=$childPid;helperStartUtc=$childStartUtc;helperExitCode=$childExitCode;helperHandleRetained=($null -ne $childHandle);expectedRect=$ExpectedRect;input=$inputIdentity;readyUtc=$readyUtc;firstFrame=$firstFrame;burstOutcome=$burstOutcome;inputBeforeUtc=$inputBeforeUtc;inputDownUtc=$inputDownUtc;inputUpUtc=$inputUpUtc;downSendInputResult=$downSendInputResult;upSendInputResult=$upSendInputResult;clickAttempted=$clickAttempted;chronology=[ordered]@{firstFrameAfterController=($null -ne $firstFrame -and [DateTime]::Parse($firstFrame.captureStartedUtc).ToUniversalTime() -ge [DateTime]::Parse($controllerStartUtc).ToUniversalTime());inputAfterReady=($null -ne $readyUtc -and $null -ne $inputBeforeUtc -and [DateTime]::Parse($inputBeforeUtc).ToUniversalTime() -ge [DateTime]::Parse($readyUtc).ToUniversalTime());inputWhileHelperAlive=$childAliveAtInput;inputInsideAcceptedBurst=($burstIdentityMatch -and $burstOutcome -eq 'completed' -and $null -ne $inputDownUtc -and $null -ne $inputUpUtc -and [DateTime]::Parse($inputDownUtc).ToUniversalTime() -ge [DateTime]::Parse($burst.captureStartedUtc).ToUniversalTime() -and [DateTime]::Parse($inputUpUtc).ToUniversalTime() -le [DateTime]::Parse($burst.captureEndedUtc).ToUniversalTime())};outcome=$outcome;failure=$failure;sceneState='not inferred from capture';performanceAcceptance=$false}|ConvertTo-Json -Depth 12|Set-Content -LiteralPath $controllerReceipt -Encoding UTF8;$receiptWritten=$true}
 if($null -ne $child){$child.Dispose()}
}
if($receiptWritten){Get-Content -LiteralPath $controllerReceipt -Raw}
if($outcome -eq 'failed'){throw ('Synchronized rebuild controller failed: '+$failure)}
