param([Parameter(Mandatory=$true)][ValidateSet('baseline','candidate')][string]$BuildRole,[Parameter(Mandatory=$true)][ValidateSet('focused-one')][string]$Mode,[Parameter(Mandatory=$true)][ValidatePattern('^[A-Za-z0-9_-]+$')][string]$CellId,[Parameter(Mandatory=$true)][string]$BuildManifest,[Parameter(Mandatory=$true)][string]$BuildReceipt,[Parameter(Mandatory=$true)][string]$StimulusPlan,[Parameter(Mandatory=$true)][string]$StimulusReceipt,[Parameter(Mandatory=$true)][string]$PrepareReceipt,[string]$HostRoot=(Join-Path $env:USERPROFILE '274bot-workspaces\3118e96\host') )
$ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue'
$env:COHORT_REFERENCE_STAGE=if($env:COHORT_REFERENCE_STAGE){$env:COHORT_REFERENCE_STAGE}else{'C:\ProgramData\274bot-Test\cohort-reference-e25f328'}; $env:COHORT_CANDIDATE_STAGE=if($env:COHORT_CANDIDATE_STAGE){$env:COHORT_CANDIDATE_STAGE}else{'C:\ProgramData\274bot-Test\cohort-candidate-ca56e143'}
$stage=if($BuildRole -eq 'baseline'){$env:COHORT_REFERENCE_STAGE}else{$env:COHORT_CANDIDATE_STAGE}
$prefix="$BuildRole-$Mode-"; if(-not $CellId.StartsWith($prefix)){throw "CellId must start with $prefix"}
$runner=Join-Path $PSScriptRoot 'run-cohort-pair.py'
$preflightDir=if($env:COHORT_PREFLIGHT_DIR){$env:COHORT_PREFLIGHT_DIR}else{'C:\ProgramData\274bot-Test\cohort-preflight'}; $preflight=Join-Path $preflightDir ('preflight-cohort-'+$CellId+'.json')
if(-not (Test-Path $runner)){throw 'Controller missing'}; if(-not (Test-Path (Join-Path $stage 'panel-play.exe'))){throw 'Role binary missing'}; if(-not (Test-Path $preflight)){throw 'Fresh per-cell preflight missing'}
$prepare = Get-Content $PrepareReceipt -Raw | ConvertFrom-Json
if($prepare.schema -ne 'cohort-pair-prepare-no-launch-v1' -or $prepare.cell_id -ne $CellId -or $prepare.client_started -ne $false -or $prepare.scheduled_task_created -ne $false){throw 'Exact no-launch prepare receipt is required before scheduling'}
$out=Join-Path 'C:\Users\BotTest\274bot-runs' ('managed-cohort-pair-'+$CellId); if(Test-Path $out){throw 'Cell output exists; choose a new cell id'}
Copy-Item $runner (Join-Path $stage ("run-"+$CellId+'.py')) -Force
$launcher=Join-Path $stage ("launch-"+$CellId+'.ps1'); $log=Join-Path 'C:\Users\BotTest\274bot-runs' ('managed-cohort-pair-'+$CellId+'-launcher.log')
@"
`$ErrorActionPreference='Continue'; `$ProgressPreference='SilentlyContinue'
`$env:COHORT_BUILD_ROLE='$BuildRole'; `$env:COHORT_MODE='$Mode'; `$env:COHORT_CELL_ID='$CellId'; `$env:COHORT_HOST_ROOT='$HostRoot'; `$env:COHORT_REFERENCE_STAGE='$($env:COHORT_REFERENCE_STAGE)'; `$env:COHORT_CANDIDATE_STAGE='$($env:COHORT_CANDIDATE_STAGE)'; `$env:COHORT_PREFLIGHT_DIR='$preflightDir'; `$env:COHORT_BUILD_MANIFEST='$BuildManifest'; `$env:COHORT_BUILD_RECEIPT='$BuildReceipt'; `$env:COHORT_STIMULUS_PLAN='$StimulusPlan'; `$env:COHORT_STIMULUS_RECEIPT='$StimulusReceipt'; `$env:COHORT_PREPARE_RECEIPT='$PrepareReceipt'; Remove-Item Env:BOT_DEBUG -ErrorAction SilentlyContinue; Remove-Item Env:BOT_INPUT_SEAM_TRACE -ErrorAction SilentlyContinue
& 'C:\Program Files\Python314\python.exe' '$stage\run-$CellId.py' *> '$log'; exit `$LASTEXITCODE
"@|Set-Content $launcher -Encoding UTF8
$name='274bot-BotTest-cohort-pair-'+$CellId; if(Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue){throw 'Task exists; preserve prior output'}
$action=New-ScheduledTaskAction -Execute 'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe' -Argument ('-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "'+$launcher+'"') -WorkingDirectory $stage
$principal=New-ScheduledTaskPrincipal -UserId 'DESKTOP-SL99R6C\BotTest' -LogonType Interactive -RunLevel Limited; $settings=New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 18)
Register-ScheduledTask -TaskName $name -Action $action -Principal $principal -Settings $settings|Out-Null; Start-ScheduledTask -TaskName $name
[ordered]@{task=$name;cell_id=$CellId;build_role=$BuildRole;mode=$Mode;count=16;observe_s=600;warmup_s=120;cohort_tail_s=5;teardown_grace_s=60;requestedBackend='gpu';adapter='Intel(R) Graphics';runnerSha256=(Get-FileHash $runner -Algorithm SHA256).Hash.ToLower();startedUtc=[DateTime]::UtcNow.ToString('o');performanceAcceptance=$false}|ConvertTo-Json
