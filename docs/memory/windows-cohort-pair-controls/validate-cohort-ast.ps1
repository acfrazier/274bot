param([string]$ControlDirectory = $PSScriptRoot)
$ErrorActionPreference = 'Stop'
$controls = @('prepare-cohort.ps1','stage-contract.ps1','run-contractcheck.ps1','preflight-cohort.ps1','launch-cohort.ps1','poll-cohort.ps1','archive-cohort.ps1')
foreach($name in $controls) {
    $path = Join-Path $ControlDirectory $name
    if(-not (Test-Path $path -PathType Leaf)){ throw "Control missing: $path" }
    $tokens = $null; $errors = $null
    [System.Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$errors) | Out-Null
    if($errors.Count -ne 0){ throw "PowerShell parse failed for ${name}: $((@($errors | ForEach-Object { $_.Message })) -join '; ')" }
    if($name -ne 'validate-cohort-ast.ps1' -and (Get-Content $path -Raw) -match 'TILE_BOXED_|tile-boxed|managed-tile'){ throw "Legacy namespace leaked into cohort control: $name" }
    "PASS $name"
}