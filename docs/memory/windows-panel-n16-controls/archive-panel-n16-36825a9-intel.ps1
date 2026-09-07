param([Parameter(Mandatory=$true)][ValidateSet('focused-one','focused-plus-background')][string]$Mode,[Parameter(Mandatory=$true)][string]$Destination)
$ErrorActionPreference='Stop'; $label='n16-'+$Mode+'-console-intel'; $run=Join-Path 'C:\Users\BotTest' ('274bot-runs\managed-panel-36825a9-'+$label)
if(-not (Test-Path $run)){throw 'Run directory missing'}; $completion=Join-Path $run 'completion.json'; if(-not (Test-Path $completion)){throw 'Completion receipt missing'}
$c=Get-Content $completion -Raw|ConvertFrom-Json; if($c.exit_code -ne 0){throw ('Refusing archive of failed process: '+$c.exit_code)}
$dest=Join-Path $Destination $label; if(Test-Path $dest){throw 'Archive destination exists; preserve prior output'}; New-Item -ItemType Directory -Path $dest|Out-Null
Copy-Item $run -Destination $dest -Recurse -Force
$files=Get-ChildItem $dest -File -Recurse|Sort-Object FullName|ForEach-Object{[ordered]@{path=$_.FullName.Substring($dest.Length+1);sha256=(Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower();length=$_.Length}}
[ordered]@{mode=$Mode;count=16;adapter='Intel(R) Graphics';backgroundFps=if($Mode -eq 'focused-plus-background'){1}else{$null};sourceRun=$run;archive=$dest;files=$files;performanceAcceptance=$false}|ConvertTo-Json -Depth 8|Set-Content (Join-Path $dest 'archive-manifest.json') -Encoding UTF8
Get-FileHash (Join-Path $dest 'archive-manifest.json') -Algorithm SHA256
