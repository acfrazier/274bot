# Windows nightly host: the action of the `274bot-nightly` and
# `274bot-nightly-catchup` scheduled tasks (see tools/nightly/README.md).
# Runs as BotTest without elevation: it reads C:\274bot-nightly\nightly.env,
# puts the toolchain on PATH and starts the main and quest jobs separately
# with the nightly venv's Python. Output goes to logs\<stamp>-<job>-<mode>.log/.err.log.
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
$Python = Join-Path $Root 'venv\Scripts\python.exe'
$Runner = Join-Path $Root 'runner\tools\nightly\run_nightly.py'
$Revision = if ($env:NIGHTLY_REVISION) { $env:NIGHTLY_REVISION } else { '289' }
$RevisionSuffix = if ($Revision -eq '289') { '' } else { "-$Revision" }
$ExitCode = 0
foreach ($Job in @('main', 'quest')) {
    $env:NIGHTLY_JOB = $Job
    $env:NIGHTLY_ENGINE = 'remote'
    $env:NIGHTLY_ENGINE_HOST = '127.0.0.1'
    $env:NIGHTLY_GAME_PORT = '47594'
    $env:NIGHTLY_HTTP_PORT = '3080'
    $env:NIGHTLY_ENGINE_LABEL = 'Engine B'
    Remove-Item Env:QUESTER_TICK_MS -ErrorAction SilentlyContinue
    if ($Job -eq 'quest') {
        $env:NIGHTLY_STEPS = 'panel,tui'
        $env:NIGHTLY_RUN_SUFFIX = "$RevisionSuffix-windows-quest"
        $env:NIGHTLY_NAME_PREFIX = 'wq'
        $env:NIGHTLY_PANEL_NAME_PREFIX = 'wqp'
    } else {
        $env:NIGHTLY_STEPS = 'lanes,panel,tui'
        $env:NIGHTLY_RUN_SUFFIX = "$RevisionSuffix-windows"
        $env:NIGHTLY_NAME_PREFIX = 'nw'
        $env:NIGHTLY_PANEL_NAME_PREFIX = 'np'
    }
    $Out = Join-Path $Logs "$Stamp-$Job-$Mode.log"
    $Err = Join-Path $Logs "$Stamp-$Job-$Mode.err.log"
    $Arguments = @('-u', "`"$Runner`"")
    if ($Mode -eq 'catch-up') { $Arguments += '--catch-up' }
    $Process = Start-Process -FilePath $Python -ArgumentList $Arguments -WorkingDirectory $Root -NoNewWindow -Wait `
        -PassThru -RedirectStandardOutput $Out -RedirectStandardError $Err
    if ($Process.ExitCode -ne 0) { $ExitCode = $Process.ExitCode }
}
exit $ExitCode
