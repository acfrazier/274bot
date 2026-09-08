param([Parameter(Mandatory=$true)][string]$ControlDirectory)
$ErrorActionPreference='Stop'; $files=Get-ChildItem $ControlDirectory -Filter *.ps1; foreach($f in $files){$tokens=$null; $errors=$null; [System.Management.Automation.Language.Parser]::ParseFile($f.FullName,[ref]$tokens,[ref]$errors); if($errors.Count){throw "PowerShell parse failed: $($f.Name)"}}; 'PowerShell AST validation passed'
