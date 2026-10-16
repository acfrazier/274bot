# Windows nightly host: the action of the `274bot-nightly` and
# `274bot-nightly-catchup` scheduled tasks (see tools/nightly/README.md).
# Runs as BotTest without elevation: it reads C:\274bot-nightly\nightly.env,
# puts the toolchain on PATH and starts run_nightly.py from the runner checkout
# with the nightly venv's Python. Output goes to logs\<stamp>-<mode>.log/.err.log.
param(
    [ValidateSet('night', 'catch-up')]
    [string]$Mode = 'night'
)
$ErrorActionPreference = 'Stop'
$Root = if ($env:NIGHTLY_ROOT) { $env:NIGHTLY_ROOT } else { 'C:\274bot-nightly' }
$EnvFile = Join-Path $Root 'nightly.env'
if (Test-Path -LiteralPath $EnvFile) {
    foreach ($line in [IO.File]::ReadAllLines($EnvFile)) {
        if ($line -match '^\s*([A-Za-z_][A-Za-z0-9_]*)=(.*)$') {
            [Environment]::SetEnvironmentVariable($Matches[1], $Matches[2].Trim(), 'Process')
        }
    }
}
if ($env:NIGHTLY_PATH_PREPEND) { $env:PATH = $env:NIGHTLY_PATH_PREPEND + ';' + $env:PATH }
$env:NIGHTLY_ROOT = $Root
$env:PYTHONUTF8 = '1'
$env:PYTHONIOENCODING = 'utf-8'
$Logs = Join-Path $Root 'logs'
New-Item -ItemType Directory -Force -Path $Logs | Out-Null
# Task logs older than 30 days go; runs are pruned by the runner itself.
Get-ChildItem -LiteralPath $Logs -File | Where-Object { $_.LastWriteTime -lt (Get-Date).AddDays(-30) } |
    Remove-Item -Force -ErrorAction SilentlyContinue
$Stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$Out = Join-Path $Logs "$Stamp-$Mode.log"
$Err = Join-Path $Logs "$Stamp-$Mode.err.log"
$Python = Join-Path $Root 'venv\Scripts\python.exe'
$Runner = Join-Path $Root 'runner\tools\nightly\run_nightly.py'
$Arguments = @('-u', "`"$Runner`"")
if ($Mode -eq 'catch-up') { $Arguments += '--catch-up' }
$Process = Start-Process -FilePath $Python -ArgumentList $Arguments -WorkingDirectory $Root -NoNewWindow -Wait `
    -PassThru -RedirectStandardOutput $Out -RedirectStandardError $Err
exit $Process.ExitCode
