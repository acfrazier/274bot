param(
    [Parameter(Mandatory=$true)]
    [ValidatePattern('^native-render-owner-census-focused-plus-background-[A-Za-z0-9_.-]+$')]
    [string]$CellId,
    [string]$HostRoot = (Join-Path $env:USERPROFILE '274bot-workspaces\3118e96\host')
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
if($env:USERNAME -ne 'BotTest'){ throw 'Contract check must run as BotTest Interactive Limited' }
$contractId = $CellId + '-contractcheck-tile'
$stage = 'C:\ProgramData\274bot-Test\renderer-owner-census-9268890'
$preflight = Join-Path $stage ('preflight-tile-probe-'+$contractId+'.json')
if(-not (Test-Path $preflight -PathType Leaf)){ throw "Privileged host preflight receipt missing: $preflight" }
Copy-Item (Join-Path $PSScriptRoot 'run-panel-tile-probe-focused-one.py') (Join-Path $stage 'run-panel-tile-probe-focused-one.py') -Force
$python = 'C:\Program Files\Python314\python.exe'
$env:RENDER_OWNER_CENSUS_CONTRACT_ID = $contractId
$env:RENDER_OWNER_CENSUS_CELL_ID = $CellId
$env:RENDER_OWNER_CENSUS_HOST_ROOT = $HostRoot
Push-Location $HostRoot
try { & $python (Join-Path $PSScriptRoot 'check-tile-probe-contract.py') }
finally { Pop-Location }
if($LASTEXITCODE -ne 0){ throw "Native no-launch contract failed: $LASTEXITCODE" }
$receipt = Join-Path ([Environment]::GetFolderPath('UserProfile')) ('274bot-runs\owner-contractcheck-tile-probe-'+$contractId+'.json')
if(-not (Test-Path $receipt -PathType Leaf)){ throw "Contract receipt missing: $receipt" }
Get-Content $receipt -Raw