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
    [string]$HostRoot = (Join-Path $env:USERPROFILE '274bot-workspaces\3118e96\host'),
    [string]$ContractReceipt,
    [Parameter(Mandatory=$true)][string]$BuildManifest,
    [Parameter(Mandatory=$true)][string]$BuildReceipt,
    [Parameter(Mandatory=$true)][string]$StimulusPlan,
    [Parameter(Mandatory=$true)][string]$StimulusReceipt
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$controls = $PSScriptRoot
$env:COHORT_BUILD_ROLE = $BuildRole
$env:COHORT_MODE = $Mode
$env:COHORT_CELL_ID = $CellId
$env:COHORT_HOST_ROOT = $HostRoot
$env:COHORT_BUILD_MANIFEST = $BuildManifest
$env:COHORT_BUILD_RECEIPT = $BuildReceipt
$env:COHORT_STIMULUS_PLAN = $StimulusPlan
$env:COHORT_STIMULUS_RECEIPT = $StimulusReceipt
if(-not $ContractReceipt){ $ContractReceipt = 'C:\Users\BotTest\274bot-runs\cohort-contract-' + $CellId + '-cohort-contractcheck.json' }

# This is the privileged consume half of the no-launch gate. AST validation,
# preflight, and contract staging have already run in their documented order;
# the real controller contract check ran separately as BotTest Interactive
# Limited.
$stage = if($BuildRole -eq 'baseline'){if($env:COHORT_REFERENCE_STAGE){$env:COHORT_REFERENCE_STAGE}else{'C:\ProgramData\274bot-Test\cohort-reference-e25f328'}}else{if($env:COHORT_CANDIDATE_STAGE){$env:COHORT_CANDIDATE_STAGE}else{'C:\ProgramData\274bot-Test\cohort-candidate-ca56e143'}}
$preflightDir=if($env:COHORT_PREFLIGHT_DIR){$env:COHORT_PREFLIGHT_DIR}else{'C:\ProgramData\274bot-Test\cohort-preflight'}; $preflight = Join-Path $preflightDir ('preflight-cohort-'+$CellId+'.json')
if(-not (Test-Path $preflight -PathType Leaf)){ throw 'No-launch preflight receipt missing' }
$record = Get-Content $preflight -Raw | ConvertFrom-Json
if($record.cell_id -ne $CellId){ throw 'Preflight cell binding mismatch' }
[string]$expectedStage = $stage
if($record.stages | Where-Object {$_.role -eq $BuildRole -and $_.path -ne $expectedStage}){ throw 'Role stage binding mismatch' }
[string]$contractReceiptPath = $ContractReceipt
if(-not (Test-Path $contractReceiptPath -PathType Leaf)){ throw "BotTest contract receipt missing: $contractReceiptPath" }
foreach($path in @($BuildManifest,$BuildReceipt,$StimulusPlan)){ if(-not (Test-Path $path -PathType Leaf)){throw "Required provenance file missing: $path"} }
$contract = Get-Content $contractReceiptPath -Raw | ConvertFrom-Json
if($contract.kind -ne 'no-launch contract test' -or $contract.performance_acceptance -ne $false){ throw 'Invalid no-launch contract receipt' }
$expectedContractId = $CellId + '-cohort-contractcheck'
if($contract.cell_id -ne $CellId -or $contract.contract_id -ne $expectedContractId -or $contract.check_id -ne $CellId){ throw 'No-launch contract receipt identity mismatch' }
if(@($contract.checks).Count -ne 1 -or $contract.checks[0].id -ne $CellId -or $contract.checks[0].contract_cell_id -ne $expectedContractId -or $contract.checks[0].launched -ne $false -or $contract.checks[0].client_started -ne $false -or $contract.checks[0].native_conditions_complete -ne $true){ throw 'No-launch contract evidence is incomplete' }
[ordered]@{
    schema = 'cohort-pair-prepare-no-launch-v1'
    cell_id = $CellId
    host_root = $HostRoot
    stage = $stage
    ast_parser = 'consumed before privileged staging'
    native_preflight = 'consumed before privileged staging'
    python_contract_tests = 'run separately as BotTest Interactive Limited'
    binding_conditions = 'passed'
    count = 16
    mode = 'focused-one'
    observe_s = 600
    warmup_s = 120
    cohort_tail_s = 5
    teardown_grace_s = 60
    requested_backend = 'gpu'
    client_started = $false
    scheduled_task_created = $false
    performanceAcceptance = $false
    build_manifest = $BuildManifest
    build_receipt = $BuildReceipt
    stimulus_plan = $StimulusPlan
    stimulus_receipt = $StimulusReceipt
}|ConvertTo-Json -Depth 5
