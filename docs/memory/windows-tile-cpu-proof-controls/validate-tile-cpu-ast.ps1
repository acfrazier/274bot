param([Parameter(Mandatory=$true)][string]$ControlDirectory)
$ErrorActionPreference='Stop'; $files=Get-ChildItem $ControlDirectory -Filter *.ps1; foreach($f in $files){[System.Management.Automation.Language.Parser]::ParseFile($f.FullName,[ref]$null,[ref]$errors); if($errors.Count){throw "PowerShell parse failed: $($f.Name)"}}; 'PowerShell AST validation passed'
