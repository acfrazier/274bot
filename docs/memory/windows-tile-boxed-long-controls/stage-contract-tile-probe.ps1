param(
    [Parameter(Mandatory=$true)][ValidateSet('baseline','candidate')][string]$BuildRole,
    [Parameter(Mandatory=$true)][ValidateSet('focused-one')][string]$Mode,
    [Parameter(Mandatory=$true)][ValidatePattern('^[A-Za-z0-9_-]+$')][string]$CellId
)
$ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue'
if($env:USERNAME -eq 'BotTest'){throw 'Contract staging must run in the privileged host context'}
$prefix="$BuildRole-$Mode-"; if(-not $CellId.StartsWith($prefix)){throw "CellId must start with $prefix"}
$stage=if($BuildRole -eq 'baseline'){'C:\ProgramData\274bot-Test\renderer-owner-census-9268890'}else{'C:\ProgramData\274bot-Test\tile-boxed-fb3589a'}
$preflight=Join-Path 'C:\ProgramData\274bot-Test\renderer-owner-census-9268890' ('preflight-tile-boxed-long-'+$CellId+'.json')
$source=Join-Path $PSScriptRoot 'run-tile-boxed-focused-one.py'
$contractId=$CellId+'-long-contractcheck'
$destination=Join-Path $stage ('run-'+$contractId+'.py')
if(-not (Test-Path $source -PathType Leaf)){throw "Controller missing: $source"}
if(-not (Test-Path $stage -PathType Container)){throw "Role stage missing: $stage"}
if(-not (Test-Path (Join-Path $stage 'panel-play.exe') -PathType Leaf)){throw "Role binary missing: $stage"}
if(-not (Test-Path $preflight -PathType Leaf)){throw "Privileged preflight receipt missing: $preflight"}
Copy-Item $source $destination -Force
$sourceSha=(Get-FileHash $source -Algorithm SHA256).Hash.ToLower()
$stagedSha=(Get-FileHash $destination -Algorithm SHA256).Hash.ToLower()
if($sourceSha -ne $stagedSha){throw 'Staged contract runner hash does not match controls source'}
[ordered]@{schema='tile-boxed-long-contract-stage-v1';cell_id=$CellId;contract_id=$contractId;count=16;mode='focused-one';observe_s=600;warmup_s=30;teardown_grace_s=60;requestedBackend='gpu';source=$source;destination=$destination;sourceSha256=$sourceSha;stagedSha256=$stagedSha;privileged=$true;performanceAcceptance=$false}|ConvertTo-Json -Depth 5
