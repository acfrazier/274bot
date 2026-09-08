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
    [string]$HostRoot = (Join-Path $env:USERPROFILE '274bot-workspaces\3118e96\host'),
    [string]$ContractReceipt
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$controls = $PSScriptRoot
$env:TILE_BOXED_BUILD_ROLE = $BuildRole
$env:TILE_BOXED_MODE = $Mode
$env:TILE_BOXED_CELL_ID = $CellId
$env:TILE_BOXED_HOST_ROOT = $HostRoot
if(-not $ContractReceipt){ $ContractReceipt = 'C:\Users\BotTest\274bot-runs\tile-boxed-contract-' + $CellId + '-contractcheck.json' }

# This is the privileged host half of the no-launch gate.  Get-VM and the
# native preflight must run in Austen's SSH/admin context; the real controller
# contract check is run separately as BotTest Interactive Limited.
& (Join-Path $controls 'validate-tile-boxed-ast.ps1') -ControlDirectory $controls
& (Join-Path $controls 'preflight-tile-boxed.ps1') -CellId $CellId
$stage = if($BuildRole -eq 'baseline'){'C:\ProgramData\274bot-Test\renderer-owner-census-9268890'}else{'C:\ProgramData\274bot-Test\tile-boxed-fb3589a'}
$preflight = Join-Path 'C:\ProgramData\274bot-Test\renderer-owner-census-9268890' ('preflight-tile-boxed-'+$CellId+'.json')
if(-not (Test-Path $preflight -PathType Leaf)){ throw 'No-launch preflight receipt missing' }
$record = Get-Content $preflight -Raw | ConvertFrom-Json
if($record.cell_id -ne $CellId){ throw 'Preflight cell binding mismatch' }
[string]$expectedStage = $stage
if($record.stages | Where-Object {$_.role -eq $BuildRole -and $_.path -ne $expectedStage}){ throw 'Role stage binding mismatch' }
[string]$contractReceiptPath = $ContractReceipt
if(-not (Test-Path $contractReceiptPath -PathType Leaf)){ throw "BotTest contract receipt missing: $contractReceiptPath" }
$contract = Get-Content $contractReceiptPath -Raw | ConvertFrom-Json
if($contract.kind -ne 'no-launch contract test' -or $contract.performance_acceptance -ne $false){ throw 'Invalid no-launch contract receipt' }
if(@($contract.checks).Count -ne 1 -or $contract.checks[0].launched -ne $false -or $contract.checks[0].client_started -ne $false -or $contract.checks[0].native_conditions_complete -ne $true){ throw 'No-launch contract evidence is incomplete' }
[ordered]@{
    schema = 'tile-boxed-prepare-no-launch-v2'
    cell_id = $CellId
    host_root = $HostRoot
    stage = $stage
    ast_parser = 'passed'
    native_preflight = 'passed'
    python_contract_tests = 'run separately as BotTest Interactive Limited'
    binding_conditions = 'passed'
    client_started = $false
    scheduled_task_created = $false
    performanceAcceptance = $false
}|ConvertTo-Json -Depth 5
