param(
    [Parameter(Mandatory=$true)]
    [ValidateSet('baseline','candidate')]
    [string]$BuildRole,
    [Parameter(Mandatory=$true)]
    [ValidateSet('focused-one','focused-plus-background')]
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
$contractId = $CellId + '-contractcheck'
$stage = if($BuildRole -eq 'baseline'){'C:\ProgramData\274bot-Test\renderer-owner-census-9268890'}else{'C:\ProgramData\274bot-Test\tile-boxed-fb3589a'}
$preflight = Join-Path 'C:\ProgramData\274bot-Test\renderer-owner-census-9268890' ('preflight-tile-boxed-'+$CellId+'.json')
if(-not (Test-Path $preflight -PathType Leaf)){ throw "Privileged host preflight receipt missing: $preflight" }
$contractCellId = $CellId + '-contractcheck'
Copy-Item (Join-Path $PSScriptRoot 'run-tile-boxed-focused-one.py') (Join-Path $stage ("run-$contractCellId.py")) -Force
$python = 'C:\Program Files\Python314\python.exe'
$env:TILE_BOXED_CONTRACT_ID = $contractId
$env:TILE_BOXED_BUILD_ROLE = $BuildRole
$env:TILE_BOXED_MODE = $Mode
$env:TILE_BOXED_CELL_ID = $contractCellId
$env:TILE_BOXED_TARGET_CELL_ID = $CellId
$env:TILE_BOXED_PREFLIGHT_CELL_ID = $CellId
$env:TILE_BOXED_HOST_ROOT = $HostRoot
Push-Location $HostRoot
try { & $python (Join-Path $PSScriptRoot 'check-tile-probe-contract.py') }
finally { Pop-Location }
if($LASTEXITCODE -ne 0){ throw "Native no-launch contract failed: $LASTEXITCODE" }
$receipt = Join-Path ([Environment]::GetFolderPath('UserProfile')) ('274bot-runs\tile-boxed-contract-'+$contractId+'.json')
if(-not (Test-Path $receipt -PathType Leaf)){ throw "Contract receipt missing: $receipt" }
Get-Content $receipt -Raw