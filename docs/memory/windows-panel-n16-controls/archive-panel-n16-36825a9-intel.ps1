param([Parameter(Mandatory=$true)][ValidateSet('focused-one','focused-plus-background')][string]$Mode,[Parameter(Mandatory=$true)][string]$Destination)
$ErrorActionPreference='Stop'; $label='n16-'+$Mode+'-console-intel'; $run=Join-Path 'C:\Users\BotTest' ('274bot-runs\managed-panel-36825a9-'+$label)
if(-not (Test-Path $run)){throw 'Run directory missing'}; $completion=Join-Path $run 'completion.json'; if(-not (Test-Path $completion)){throw 'Completion receipt missing'}
$receipt=Get-Content $completion -Raw|ConvertFrom-Json
$dest=Join-Path $Destination $label; if(Test-Path $dest){throw 'Archive destination exists; preserve prior output'}; New-Item -ItemType Directory -Path $dest|Out-Null
$managed=Join-Path $dest 'managed-run'; New-Item -ItemType Directory -Path $managed|Out-Null; Copy-Item (Join-Path $run '*') -Destination $managed -Recurse -Force
$rawSources=@(); $receipts=@(Get-ChildItem $run -Recurse -File -Filter receipt.json -ErrorAction SilentlyContinue)
foreach($receiptPath in $receipts){
    $r=Get-Content $receiptPath.FullName -Raw|ConvertFrom-Json
    if($r.run_dir){$rawSources += [string]$r.run_dir}
}
if($receipt.run_dir){$rawSources += [string]$receipt.run_dir}
$rawSources=@($rawSources|Sort-Object -Unique)
$rawIndex=0; $rawRecords=@()
foreach($raw in $rawSources){
    if(Test-Path $raw -PathType Container){$rawIndex++; $rawDest=Join-Path $dest ('raw-run-{0:d2}' -f $rawIndex); New-Item -ItemType Directory -Path $rawDest|Out-Null; Copy-Item (Join-Path $raw '*') -Destination $rawDest -Recurse -Force; $rawRecords += [ordered]@{source=$raw;archive=$rawDest;present=$true}}
    else{$rawRecords += [ordered]@{source=$raw;archive=$null;present=$false}}
}
$launcherLog=Join-Path 'C:\Users\BotTest' ('274bot-runs\managed-panel-36825a9-'+$label+'-launcher.log'); $launcherRecord=$null
if(Test-Path $launcherLog -PathType Leaf){$launcherDest=Join-Path $dest 'launcher.log'; Copy-Item $launcherLog $launcherDest -Force; $launcherRecord=[ordered]@{source=$launcherLog;archive=$launcherDest}}
$files=Get-ChildItem $dest -File -Recurse|Sort-Object FullName|ForEach-Object{[ordered]@{path=$_.FullName.Substring($dest.Length+1);sha256=(Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower();length=$_.Length}}
$manifestPath=Join-Path $dest 'archive-manifest.json'
[ordered]@{mode=$Mode;count=16;adapter='Intel(R) Graphics';backgroundFps=if($Mode -eq 'focused-plus-background'){1}else{$null};sourceRun=$run;archive=$dest;receiptExitCode=$receipt.exit_code;receiptStatus=$receipt.status;rawRuns=$rawRecords;launcherLog=$launcherRecord;files=$files;performanceAcceptance=$false;note='Archive preserves failed or incomplete evidence; receipt status is not a qualification verdict.'}|ConvertTo-Json -Depth 10|Set-Content $manifestPath -Encoding UTF8
Get-FileHash $manifestPath -Algorithm SHA256
