param(
    [Parameter(Mandatory=$true)]
    [ValidatePattern('^native-render-owner-census-focused-plus-background-[A-Za-z0-9_.-]+$')]
    [string]$CellId,
    [string]$HostRoot = (Join-Path $env:USERPROFILE '274bot-workspaces\3118e96\host'),
    [string]$ContractReceipt
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$controls = $PSScriptRoot
$env:RENDER_OWNER_CENSUS_HOST_ROOT = $HostRoot
if(-not $ContractReceipt){ $ContractReceipt = Join-Path ([Environment]::GetFolderPath('UserProfile')) ('274bot-runs\owner-contractcheck-tile-probe-' + $CellId + '-contractcheck-tile.json') }

# This is the privileged host half of the no-launch gate.  Get-VM and the
# native preflight must run in Austen's SSH/admin context; the real controller
# contract check is run separately as BotTest Interactive Limited.
& (Join-Path $controls 'validate-controls-ast.ps1') -ControlDirectory $controls
& (Join-Path $controls 'preflight-tile-probe.ps1') -CellId $CellId
$contractId = $CellId + '-contractcheck-tile'
& (Join-Path $controls 'preflight-tile-probe.ps1') -CellId $contractId
$stage = 'C:\ProgramData\274bot-Test\renderer-owner-census-9268890'
$preflight = Join-Path $stage ('preflight-tile-probe-'+$CellId+'.json')
if(-not (Test-Path $preflight -PathType Leaf)){ throw 'No-launch preflight receipt missing' }
$record = Get-Content $preflight -Raw | ConvertFrom-Json
if($record.cell_id -ne $CellId){ throw 'Preflight cell binding mismatch' }
if($record.strict_binding -ne '9268890-only'){ throw 'Frozen native binding mismatch' }
if($record.binary_sha256 -ne 'e2deb1db371366f674c18e39d04f7309480310072ade224ffa1433c84d4559b5'){ throw 'Frozen binary identity mismatch' }
[string]$contractReceiptPath = $ContractReceipt
if(-not (Test-Path $contractReceiptPath -PathType Leaf)){ throw "BotTest contract receipt missing: $contractReceiptPath" }
$contract = Get-Content $contractReceiptPath -Raw | ConvertFrom-Json
if($contract.kind -ne 'no-launch contract test' -or $contract.performance_acceptance -ne $false){ throw 'Invalid no-launch contract receipt' }
if(@($contract.checks).Count -ne 1 -or $contract.checks[0].launched -ne $false -or $contract.checks[0].client_started -ne $false -or $contract.checks[0].native_conditions_complete -ne $true){ throw 'No-launch contract evidence is incomplete' }
[ordered]@{
    schema = 'renderer-tile-probe-prepare-no-launch-v2'
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
