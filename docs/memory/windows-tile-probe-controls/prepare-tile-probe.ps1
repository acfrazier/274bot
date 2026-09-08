param(
    [Parameter(Mandatory=$true)]
    [ValidatePattern('^native-render-owner-census-focused-plus-background-[A-Za-z0-9_.-]+$')]
    [string]$CellId,
    [string]$HostRoot = (Join-Path $env:USERPROFILE '274bot-workspaces\3118e96\host')
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$controls = $PSScriptRoot
$env:RENDER_OWNER_CENSUS_HOST_ROOT = $HostRoot

# This is a no-launch gate: it runs the real PowerShell parser, native preflight,
# Python spec/argv validators, and native binding-condition reader, but never
# invokes launch-tile-probe.ps1 or creates a scheduled task.
& (Join-Path $controls 'validate-controls-ast.ps1') -ControlDirectory $controls
& (Join-Path $controls 'preflight-tile-probe.ps1') -CellId $CellId
$contractId = $CellId + '-contractcheck-tile'
& (Join-Path $controls 'preflight-tile-probe.ps1') -CellId $contractId
$stage = 'C:\ProgramData\274bot-Test\renderer-owner-census-9268890'
Copy-Item (Join-Path $controls 'run-panel-tile-probe-focused-one.py') (Join-Path $stage 'run-panel-tile-probe-focused-one.py') -Force
$python = 'C:\Program Files\Python314\python.exe'
$tests = Join-Path $controls 'test_tile_probe_controls.py'
& $python $tests
if($LASTEXITCODE -ne 0){ throw "Local producer contract tests failed: $LASTEXITCODE" }
$env:RENDER_OWNER_CENSUS_CONTRACT_ID = $contractId
Push-Location $HostRoot
try { & $python (Join-Path $controls 'check-tile-probe-contract.py') }
finally { Pop-Location }
if($LASTEXITCODE -ne 0){ throw "Native no-launch contract failed: $LASTEXITCODE" }
$preflight = Join-Path $stage ('preflight-tile-probe-'+$CellId+'.json')
if(-not (Test-Path $preflight -PathType Leaf)){ throw 'No-launch preflight receipt missing' }
$record = Get-Content $preflight -Raw | ConvertFrom-Json
if($record.cell_id -ne $CellId){ throw 'Preflight cell binding mismatch' }
if($record.strict_binding -ne '9268890-only'){ throw 'Frozen native binding mismatch' }
if($record.binary_sha256 -ne 'e2deb1db371366f674c18e39d04f7309480310072ade224ffa1433c84d4559b5'){ throw 'Frozen binary identity mismatch' }
[ordered]@{
    schema = 'renderer-tile-probe-prepare-no-launch-v1'
    cell_id = $CellId
    host_root = $HostRoot
    stage = $stage
    ast_parser = 'passed'
    native_preflight = 'passed'
    python_contract_tests = 'passed'
    binding_conditions = 'passed'
    client_started = $false
    scheduled_task_created = $false
    performanceAcceptance = $false
}|ConvertTo-Json -Depth 5
