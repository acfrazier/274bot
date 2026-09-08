param([string]$ControlDirectory = $PSScriptRoot)
$ErrorActionPreference = 'Stop'
$controls = @('prepare-tile-probe.ps1','run-contractcheck-tile-probe.ps1','preflight-tile-probe.ps1','launch-tile-probe.ps1','poll-tile-probe.ps1','archive-tile-probe.ps1')
foreach($name in $controls) {
    $path = Join-Path $ControlDirectory $name
    if(-not (Test-Path $path -PathType Leaf)){ throw "Control missing: $path" }
    $tokens = $null; $errors = $null
    [System.Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$errors) | Out-Null
    if($errors.Count -ne 0){ throw "PowerShell parse failed for ${name}: $((@($errors | ForEach-Object { $_.Message })) -join '; ')" }
    "PASS $name"
}
