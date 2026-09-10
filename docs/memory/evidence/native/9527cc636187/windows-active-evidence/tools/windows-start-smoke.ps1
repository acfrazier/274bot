param([Parameter(Mandatory=$true)][string]$Spec,[Parameter(Mandatory=$true)][string]$RunLog)
$ErrorActionPreference='Continue';$ProgressPreference='SilentlyContinue'
if(Test-Path $RunLog){throw 'Preserve earlier launcher log'}
& 'C:\Program Files\Python314\python.exe' (Join-Path $PSScriptRoot 'windows-smoke.py') $Spec *> $RunLog
exit $LASTEXITCODE
