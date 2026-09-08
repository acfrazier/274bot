param(
 [Parameter(Mandatory=$true)][int]$PanelPid,
 [Parameter(Mandatory=$true)][string]$ExpectedStartUtc,
 [Parameter(Mandatory=$true)][string]$OutputDirectory,
 [Parameter(Mandatory=$true)][ValidatePattern('^[A-Za-z0-9_-]+$')][string]$Label,
 [ValidateRange(1,30)][int]$DurationSeconds = 5,
 [ValidateRange(1,300)][int]$MaxFrames = 60,
 [ValidateRange(10,1000)][int]$DelayMilliseconds = 100
)
$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue'

$expectedBinary='C:\ProgramData\274bot-Test\tile-boxed-fb3589a\panel-play.exe'
$expectedHash='a9b581bab780d2868035941d799d416f0ef9ec68d5eb5a4fe0a62fa29670667f'
$out=[IO.Path]::GetFullPath($OutputDirectory)
$prefix=[IO.Path]::GetFullPath('C:\Users\BotTest\274bot-runs\visual-proof-')
if($env:USERNAME -ne 'BotTest'){throw 'Capture must run in the BotTest interactive session'}
if(-not(Test-Path $out -PathType Container)){throw 'Prepared output directory missing'}
if(-not $out.StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase)){throw 'Output must be in a dedicated visual-proof directory'}
$burstReceipt=Join-Path $out ($Label+'.burst.json')
$readyMarker=Join-Path $out ($Label+'.ready')
$existingFrame=Get-ChildItem -LiteralPath $out -Filter ($Label+'-frame-*') -Force -ErrorAction SilentlyContinue | Select-Object -First 1
if((Test-Path $burstReceipt) -or (Test-Path $readyMarker) -or $null -ne $existingFrame){throw 'Burst label already exists'}

$captureStarted=$null;$captureEnded=$null;$frameCount=0;$outcome='not-started';$failure=$null
$frameReceipts=New-Object System.Collections.Generic.List[object]
$process=$null;$window=[IntPtr]::Zero;$initialRect=$null;$binaryHash=$null;$sessionId=$null
try {
 $process=Get-Process -Id $PanelPid -ErrorAction Stop
 if($process.ProcessName -ne 'panel-play' -or $process.Path -ne $expectedBinary){throw 'Unexpected target executable'}
 if($process.StartTime.ToUniversalTime().ToString('o') -ne $ExpectedStartUtc){throw 'Target process start identity changed'}
 $sessionId=$process.SessionId
 if($sessionId -ne (Get-Process -Id $PID).SessionId){throw 'Target belongs to another session'}
 $binaryHash=(Get-FileHash $expectedBinary -Algorithm SHA256).Hash.ToLower()
 if($binaryHash -ne $expectedHash){throw 'Frozen binary hash mismatch'}

 Add-Type -AssemblyName System.Drawing
 Add-Type -TypeDefinition @'
 using System;
 using System.Runtime.InteropServices;
 public static class PanelBurstCaptureNative {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left,Top,Right,Bottom; }
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern int GetWindowThreadProcessId(IntPtr h, out int pid);
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
 }
'@
 [void][PanelBurstCaptureNative]::SetProcessDPIAware()
 if([PanelBurstCaptureNative]::GetSystemMetrics(0x1000) -ne 0){throw 'Local console required'}
 $process.Refresh();$window=$process.MainWindowHandle
 if($window -eq [IntPtr]::Zero -or [PanelBurstCaptureNative]::IsIconic($window)){throw 'Target window absent or minimized'}
 $ownerPid=0;[void][PanelBurstCaptureNative]::GetWindowThreadProcessId($window,[ref]$ownerPid)
 if($ownerPid -ne $PanelPid){throw 'Target window identity changed'}
 if([PanelBurstCaptureNative]::GetForegroundWindow() -ne $window){throw 'Target window is not foreground; no other window will be captured'}
 $initialRect=New-Object PanelBurstCaptureNative+RECT
 if(-not [PanelBurstCaptureNative]::GetWindowRect($window,[ref]$initialRect)){throw 'Window bounds unavailable'}
 $width=$initialRect.Right-$initialRect.Left;$height=$initialRect.Bottom-$initialRect.Top
 if($width -le 0 -or $height -le 0 -or $width -gt 8192 -or $height -gt 8192){throw 'Invalid capture bounds'}

 $captureStarted=[DateTime]::UtcNow.ToString('o')
 $stopwatch=[Diagnostics.Stopwatch]::StartNew()
 $deadline=$DurationSeconds*1000
 while($frameCount -lt $MaxFrames -and $stopwatch.ElapsedMilliseconds -lt $deadline) {
  $process.Refresh()
  if($process.HasExited -or $process.ProcessName -ne 'panel-play' -or $process.Path -ne $expectedBinary){throw 'Target process identity changed'}
  if($process.StartTime.ToUniversalTime().ToString('o') -ne $ExpectedStartUtc -or $process.SessionId -ne $sessionId){throw 'Target process start/session identity changed'}
  if([PanelBurstCaptureNative]::GetForegroundWindow() -ne $window){throw 'Foreground changed; capture aborted'}
  if([PanelBurstCaptureNative]::IsIconic($window)){throw 'Target window became minimized'}
  $ownerPid=0;[void][PanelBurstCaptureNative]::GetWindowThreadProcessId($window,[ref]$ownerPid)
  if($ownerPid -ne $PanelPid){throw 'Target window identity changed'}
  $rect=New-Object PanelBurstCaptureNative+RECT
  if(-not [PanelBurstCaptureNative]::GetWindowRect($window,[ref]$rect)){throw 'Window bounds unavailable'}
  if($rect.Left -ne $initialRect.Left -or $rect.Top -ne $initialRect.Top -or $rect.Right -ne $initialRect.Right -or $rect.Bottom -ne $initialRect.Bottom){throw 'Window rectangle changed; capture aborted'}

  $index=$frameCount+1;$stamp=[DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ')
  $frameBase=('{0}-frame-{1:D4}-{2}' -f $Label,$index,$stamp)
  $png=Join-Path $out ($frameBase+'.png');$receipt=Join-Path $out ($frameBase+'.json')
  if((Test-Path $png) -or (Test-Path $receipt)){throw 'Frame output already exists'}
  $bitmap=New-Object Drawing.Bitmap($width,$height)
  $graphics=[Drawing.Graphics]::FromImage($bitmap)
  $frameStarted=[DateTime]::UtcNow.ToString('o')
  try {
   $graphics.CopyFromScreen($rect.Left,$rect.Top,0,0,$bitmap.Size,[Drawing.CopyPixelOperation]::SourceCopy)
   $process.Refresh()
   if($process.HasExited -or $process.ProcessName -ne 'panel-play' -or $process.Path -ne $expectedBinary){throw 'Target process identity changed during capture'}
   if($process.StartTime.ToUniversalTime().ToString('o') -ne $ExpectedStartUtc -or $process.SessionId -ne $sessionId){throw 'Target process start/session identity changed during capture'}
   if([PanelBurstCaptureNative]::GetForegroundWindow() -ne $window){throw 'Foreground changed during capture'}
   if([PanelBurstCaptureNative]::IsIconic($window)){throw 'Target window became minimized during capture'}
   $ownerPid=0;[void][PanelBurstCaptureNative]::GetWindowThreadProcessId($window,[ref]$ownerPid)
   if($ownerPid -ne $PanelPid){throw 'Target window identity changed during capture'}
   $postRect=New-Object PanelBurstCaptureNative+RECT
   if(-not [PanelBurstCaptureNative]::GetWindowRect($window,[ref]$postRect)){throw 'Window bounds unavailable during capture'}
   if($postRect.Left -ne $initialRect.Left -or $postRect.Top -ne $initialRect.Top -or $postRect.Right -ne $initialRect.Right -or $postRect.Bottom -ne $initialRect.Bottom){throw 'Window rectangle changed during capture'}
   $bitmap.Save($png,[Drawing.Imaging.ImageFormat]::Png)
  } finally {$graphics.Dispose();$bitmap.Dispose()}
  $frameEnd=[DateTime]::UtcNow.ToString('o')
  $pngHash=(Get-FileHash $png -Algorithm SHA256).Hash.ToLower()
  [ordered]@{schema='native-panel-burst-frame-v1';label=$Label;frame=$index;pid=$PanelPid;startUtc=$ExpectedStartUtc;sessionId=$sessionId;binary=$expectedBinary;binarySha256=$binaryHash;windowHandle=$window.ToInt64();rect=@($rect.Left,$rect.Top,$rect.Right,$rect.Bottom);captureStartedUtc=$frameStarted;captureEndedUtc=$frameEnd;captureUtc=$frameEnd;png=$png;pngSha256=$pngHash;sceneState='not inferred from capture';performanceAcceptance=$false}|ConvertTo-Json -Depth 5|Set-Content $receipt -Encoding UTF8
  $frameReceipts.Add([ordered]@{frame=$index;png=$png;receipt=$receipt;captureStartedUtc=$frameStarted;captureEndedUtc=$frameEnd;captureUtc=$frameEnd;pngSha256=$pngHash})
  $frameCount=$index
  if($frameCount -eq 1){if(Test-Path $readyMarker){throw 'Ready marker already exists'};Set-Content $readyMarker ('readyUtc='+[DateTime]::UtcNow.ToString('o')) -Encoding UTF8}
  $remaining=$deadline-$stopwatch.ElapsedMilliseconds
  if($remaining -gt 0 -and $frameCount -lt $MaxFrames){Start-Sleep -Milliseconds ([Math]::Min($DelayMilliseconds,$remaining))}
 }
 $outcome='completed'
 if($frameCount -eq 0){$outcome='no-frame'}
} catch {
 $outcome='failed';$failure=$_.Exception.Message
} finally {
 $captureEnded=[DateTime]::UtcNow.ToString('o')
 $durationMs=0
 if($captureStarted){$durationMs=([DateTime]::Parse($captureEnded).ToUniversalTime()-[DateTime]::Parse($captureStarted).ToUniversalTime()).TotalMilliseconds}
 $windowValue=$null;if($window -ne [IntPtr]::Zero){$windowValue=$window.ToInt64()}
 $rectValue=$null;if($initialRect){$rectValue=@($initialRect.Left,$initialRect.Top,$initialRect.Right,$initialRect.Bottom)}
 $readyValue=$null;if(Test-Path $readyMarker){$readyValue=$readyMarker}
 [ordered]@{schema='native-panel-burst-v1';label=$Label;pid=$PanelPid;startUtc=$ExpectedStartUtc;sessionId=$sessionId;binary=$expectedBinary;binarySha256=$binaryHash;windowHandle=$windowValue;rect=$rectValue;requestedDurationSeconds=$DurationSeconds;maxFrames=$MaxFrames;delayMilliseconds=$DelayMilliseconds;captureStartedUtc=$captureStarted;captureEndedUtc=$captureEnded;durationMilliseconds=$durationMs;frameCount=$frameCount;outcome=$outcome;failure=$failure;readyMarker=$readyValue;frames=$frameReceipts.ToArray();sceneState='not inferred from capture';performanceAcceptance=$false}|ConvertTo-Json -Depth 8|Set-Content $burstReceipt -Encoding UTF8
}
Get-Content $burstReceipt -Raw
if($outcome -eq 'failed' -or $outcome -eq 'no-frame'){throw ('Capture burst '+$outcome+': '+$failure)}
