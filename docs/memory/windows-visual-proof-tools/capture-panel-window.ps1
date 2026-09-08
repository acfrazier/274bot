param(
 [Parameter(Mandatory=$true)][int]$PanelPid,
 [Parameter(Mandatory=$true)][string]$ExpectedStartUtc,
 [Parameter(Mandatory=$true)][string]$OutputDirectory,
 [Parameter(Mandatory=$true)][ValidatePattern('^[A-Za-z0-9_-]+$')][string]$Label
)
$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue'
if($env:USERNAME -ne 'BotTest'){throw 'Capture must run in the BotTest interactive session'}
$expectedBinary='C:\ProgramData\274bot-Test\tile-boxed-fb3589a\panel-play.exe'
$expectedHash='a9b581bab780d2868035941d799d416f0ef9ec68d5eb5a4fe0a62fa29670667f'
$process=Get-Process -Id $PanelPid -ErrorAction Stop
if($process.ProcessName -ne 'panel-play' -or $process.Path -ne $expectedBinary){throw 'Unexpected target executable'}
if($process.StartTime.ToUniversalTime().ToString('o') -ne $ExpectedStartUtc){throw 'Target process start identity changed'}
if($process.SessionId -ne (Get-Process -Id $PID).SessionId){throw 'Target belongs to another session'}
if((Get-FileHash $expectedBinary -Algorithm SHA256).Hash.ToLower() -ne $expectedHash){throw 'Frozen binary hash mismatch'}
$out=[IO.Path]::GetFullPath($OutputDirectory)
$prefix=[IO.Path]::GetFullPath('C:\Users\BotTest\274bot-runs\visual-proof-')
if(-not $out.StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase)){throw 'Output must be in a dedicated visual-proof directory'}
if(-not(Test-Path $out -PathType Container)){throw 'Prepared output directory missing'}
$png=Join-Path $out ($Label+'.png');$receipt=Join-Path $out ($Label+'.json')
if((Test-Path $png) -or (Test-Path $receipt)){throw 'Capture label already exists'}
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class PanelCaptureNative {
 [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left,Top,Right,Bottom; }
 [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
 [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
 [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
}
'@
[void][PanelCaptureNative]::SetProcessDPIAware()
if([PanelCaptureNative]::GetSystemMetrics(0x1000) -ne 0){throw 'Local console required'}
$process.Refresh();$window=$process.MainWindowHandle
if($window -eq [IntPtr]::Zero -or [PanelCaptureNative]::IsIconic($window)){throw 'Target window absent or minimized'}
if([PanelCaptureNative]::GetForegroundWindow() -ne $window){throw 'Target window is not foreground; no other window will be captured'}
$rect=New-Object PanelCaptureNative+RECT
if(-not [PanelCaptureNative]::GetWindowRect($window,[ref]$rect)){throw 'Window bounds unavailable'}
$width=$rect.Right-$rect.Left;$height=$rect.Bottom-$rect.Top
if($width -le 0 -or $height -le 0 -or $width -gt 8192 -or $height -gt 8192){throw 'Invalid capture bounds'}
$started=[DateTime]::UtcNow.ToString('o')
$bitmap=New-Object Drawing.Bitmap($width,$height)
$graphics=[Drawing.Graphics]::FromImage($bitmap)
try {
 $graphics.CopyFromScreen($rect.Left,$rect.Top,0,0,$bitmap.Size,[Drawing.CopyPixelOperation]::SourceCopy)
 if([PanelCaptureNative]::GetForegroundWindow() -ne $window){throw 'Foreground changed during capture'}
 $bitmap.Save($png,[Drawing.Imaging.ImageFormat]::Png)
} finally {$graphics.Dispose();$bitmap.Dispose()}
$ended=[DateTime]::UtcNow.ToString('o')
[ordered]@{schema='native-panel-window-capture-v1';label=$Label;pid=$PanelPid;startUtc=$ExpectedStartUtc;sessionId=$process.SessionId;binary=$expectedBinary;binarySha256=$expectedHash;windowHandle=$window.ToInt64();rect=@($rect.Left,$rect.Top,$rect.Right,$rect.Bottom);captureStartedUtc=$started;captureEndedUtc=$ended;png=$png;pngSha256=(Get-FileHash $png -Algorithm SHA256).Hash.ToLower();sceneState='not inferred from capture';performanceAcceptance=$false}|ConvertTo-Json -Depth 5|Set-Content $receipt -Encoding UTF8
Get-Content $receipt -Raw
