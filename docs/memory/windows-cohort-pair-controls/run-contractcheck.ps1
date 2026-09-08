param(
    [Parameter(Mandatory=$true)]
    [ValidateSet('baseline','candidate')]
    [string]$BuildRole,
    [Parameter(Mandatory=$true)]
    [ValidateSet('focused-one')]
    [string]$Mode,
    [Parameter(Mandatory=$true)]
    [ValidatePattern('^[A-Za-z0-9_-]+$')]
    [string]$CellId,
    [string]$HostRoot = (Join-Path $env:USERPROFILE '274bot-workspaces\3118e96\host')
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
if($env:USERNAME -ne 'BotTest'){ throw 'Contract check must run as BotTest Interactive Limited' }
$prefix="$BuildRole-$Mode-"; if(-not $CellId.StartsWith($prefix)){throw "CellId must start with $prefix"}
$contractId = $CellId + '-cohort-contractcheck'
$stage = if($BuildRole -eq 'baseline'){if($env:COHORT_REFERENCE_STAGE){$env:COHORT_REFERENCE_STAGE}else{'C:\ProgramData\274bot-Test\cohort-reference'}}else{if($env:COHORT_CANDIDATE_STAGE){$env:COHORT_CANDIDATE_STAGE}else{'C:\ProgramData\274bot-Test\cohort-candidate'}}
$preflightDir=if($env:COHORT_PREFLIGHT_DIR){$env:COHORT_PREFLIGHT_DIR}else{'C:\ProgramData\274bot-Test\cohort-preflight'}; $preflight = Join-Path $preflightDir ('preflight-cohort-'+$CellId+'.json')
if(-not (Test-Path $preflight -PathType Leaf)){ throw "Privileged host preflight receipt missing: $preflight" }
$contractCellId = $CellId + '-cohort-contractcheck'
$runner = Join-Path $stage ("run-$contractCellId.py")
if(-not (Test-Path $runner -PathType Leaf)){ throw "Privileged staged contract runner missing: $runner" }
$source = Join-Path $PSScriptRoot 'run-cohort-pair.py'
if((Get-FileHash $runner -Algorithm SHA256).Hash.ToLower() -ne (Get-FileHash $source -Algorithm SHA256).Hash.ToLower()){ throw 'Staged contract runner hash does not match controls source' }
$python = 'C:\Program Files\Python314\python.exe'
$env:COHORT_CONTRACT_ID = $contractId
$env:COHORT_BUILD_ROLE = $BuildRole
$env:COHORT_MODE = $Mode
$env:COHORT_CELL_ID = $contractCellId
$env:COHORT_TARGET_CELL_ID = $CellId
$env:COHORT_PREFLIGHT_CELL_ID = $CellId
$env:COHORT_HOST_ROOT = $HostRoot
Push-Location $HostRoot
try { & $python (Join-Path $PSScriptRoot 'check-cohort-contract.py') }
finally { Pop-Location }
if($LASTEXITCODE -ne 0){ throw "Native no-launch contract failed: $LASTEXITCODE" }
$receipt = Join-Path ([Environment]::GetFolderPath('UserProfile')) ('274bot-runs\cohort-contract-'+$contractId+'.json')
if(-not (Test-Path $receipt -PathType Leaf)){ throw "Contract receipt missing: $receipt" }
Get-Content $receipt -Raw