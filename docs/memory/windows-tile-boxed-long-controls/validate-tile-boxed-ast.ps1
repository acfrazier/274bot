param([string]$ControlDirectory = $PSScriptRoot)
$ErrorActionPreference = 'Stop'
$controls = @('prepare-tile-probe.ps1','stage-contract-tile-probe.ps1','run-contractcheck-tile-probe.ps1','preflight-tile-boxed.ps1','launch-tile-boxed.ps1','poll-tile-boxed.ps1','archive-tile-boxed.ps1')
foreach($name in $controls) {
    $path = Join-Path $ControlDirectory $name
    if(-not (Test-Path $path -PathType Leaf)){ throw "Control missing: $path" }
    $tokens = $null; $errors = $null
    [System.Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$errors) | Out-Null
    if($errors.Count -ne 0){ throw "PowerShell parse failed for ${name}: $((@($errors | ForEach-Object { $_.Message })) -join '; ')" }
    if($name -ne 'validate-tile-boxed-ast.ps1' -and (Get-Content $path -Raw) -match 'TILE_BOXED_(?!LONG_)|managed-tile-boxed-(?!long)|tile-boxed-contract-(?!long)'){ throw "Short namespace leaked into long control: $name" }
    "PASS $name"
}