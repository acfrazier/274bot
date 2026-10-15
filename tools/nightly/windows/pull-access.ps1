# Windows nightly host, OPERATOR ONLY (elevated PowerShell): the read-only SFTP
# account the archive host pulls runs with. The nightly itself never runs this
# and needs no admin rights; this is the one host-side security change.
#
# Apply (idempotent; re-running rewrites the same state):
#   .\pull-access.ps1 -PublicKey 'ssh-ed25519 AAAA... 274bot-nightly-win-pull' [-From 10.0.0.72]
#                     [-TestPublicKey 'ssh-ed25519 AAAA... 274bot-nightly-win-pull-test' -TestFrom 10.0.0.19]
#   Re-run without -TestPublicKey to drop the test key again.
# Undo (everything this script added; the C:\274bot-nightly ACLs go back to inherited):
#   .\pull-access.ps1 -Undo
#
# What it does:
#   1. Local user 274bot-pull: no group memberships (so no interactive or RDP logon
#      right), a random password nobody knows (never printed or stored), never expires.
#   2. NTFS: C:\274bot-nightly stops inheriting C:\'s "Authenticated Users: Modify"
#      and gets BotTest/SYSTEM/Administrators full control only; runs\ adds
#      274bot-pull read and execute. The account can write nowhere in the tree.
#   3. sshd: a marked block at the end of sshd_config: AllowUsers 274bot-pull plus
#      Match User 274bot-pull with its own keys file, ChrootDirectory runs\,
#      ForceCommand internal-sftp -R (read-only), no forwarding, no TTY.
#      Validated with `sshd -t` before sshd restarts; a failing config is put back.
#   4. Keys file %ProgramData%\ssh\274bot_pull_authorized_keys (SYSTEM and
#      Administrators only), each key `restrict,from="<archive host>"`.
param(
    [string]$PublicKey,
    [string]$From = '10.0.0.72',
    [string]$TestPublicKey,
    [string]$TestFrom = '10.0.0.19',
    [switch]$Undo
)
#Requires -RunAsAdministrator
$ErrorActionPreference = 'Stop'
$User = '274bot-pull'
$Night = 'C:\274bot-nightly'
$Runs = Join-Path $Night 'runs'
$Owner = "$env:COMPUTERNAME\BotTest"
$SshDir = Join-Path $env:ProgramData 'ssh'
$Config = Join-Path $SshDir 'sshd_config'
$Keys = Join-Path $SshDir '274bot_pull_authorized_keys'
$Sshd = Join-Path $env:SystemRoot 'System32\OpenSSH\sshd.exe'
$Begin = '# BEGIN 274bot-nightly-pull'
$End = '# END 274bot-nightly-pull'

function Without-Block([string[]]$Lines) {
    $out = New-Object Collections.Generic.List[string]
    $inside = $false
    foreach ($line in $Lines) {
        if ($line -eq $Begin) { $inside = $true; continue }
        if ($line -eq $End) { $inside = $false; continue }
        if (-not $inside) { $out.Add($line) }
    }
    return , $out.ToArray()
}

function Write-Config([string[]]$Lines) {
    $backup = "$Config.274bot-pull.bak"
    Copy-Item -LiteralPath $Config -Destination $backup -Force
    [IO.File]::WriteAllLines($Config, $Lines)
    & $Sshd -t -f $Config
    if ($LASTEXITCODE -ne 0) {
        Copy-Item -LiteralPath $backup -Destination $Config -Force
        throw "sshd -t rejected the new sshd_config; the previous file is back in place"
    }
    Restart-Service sshd
    Remove-Item -LiteralPath $backup -Force
}

function Key-Line([string]$Key, [string]$Address) {
    $parts = $Key.Trim() -split '\s+'
    if ($parts.Count -lt 2 -or $parts[0] -ne 'ssh-ed25519') { throw "expected an ssh-ed25519 public key line, got: $Key" }
    if ($Address -notmatch '^\d{1,3}(\.\d{1,3}){3}$') { throw "expected an IPv4 address for from=, got: $Address" }
    return "restrict,from=`"$Address`" " + ($parts -join ' ')
}

$lines = [IO.File]::ReadAllLines($Config)
if ($Undo) {
    if (($lines -contains $Begin)) { Write-Config (Without-Block $lines) }
    if (Test-Path -LiteralPath $Keys) { Remove-Item -LiteralPath $Keys -Force }
    if (Test-Path -LiteralPath $Night) { & icacls.exe $Night /reset /T /C /Q | Out-Null }
    $account = Get-LocalUser -Name $User -ErrorAction SilentlyContinue
    if ($account) {
        Get-CimInstance Win32_UserProfile | Where-Object { $_.SID -eq $account.SID.Value } | Remove-CimInstance
        Remove-LocalUser -Name $User
    }
    "undone: sshd block, keys file, account $User and its profile removed; $Night ACLs reset to inherited"
    return
}
if (-not $PublicKey) { throw '-PublicKey is required (the archive host pull key, ssh-ed25519 ...)' }
$outside = Without-Block $lines
if ($outside | Where-Object { $_ -match '^\s*Match\s' }) {
    throw 'sshd_config already has a Match block; AllowUsers would land inside it. Add the block by hand above it.'
}
if (-not (Test-Path -LiteralPath $Runs)) { throw "$Runs does not exist (set up the nightly first)" }

# 1. Account.
if (-not (Get-LocalUser -Name $User -ErrorAction SilentlyContinue)) {
    $bytes = New-Object byte[] 48
    [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
    $password = ConvertTo-SecureString ([Convert]::ToBase64String($bytes) + 'aA1!') -AsPlainText -Force
    [Array]::Clear($bytes, 0, $bytes.Length)
    New-LocalUser -Name $User -Password $password -PasswordNeverExpires -UserMayNotChangePassword `
        -AccountNeverExpires -Description '274bot nightly: read-only SFTP pull of C:\274bot-nightly\runs' | Out-Null
}
foreach ($group in Get-LocalGroup) {
    if (Get-LocalGroupMember -Group $group -Member $User -ErrorAction SilentlyContinue) {
        Remove-LocalGroupMember -Group $group -Member $User
    }
}

# 2. NTFS.
& icacls.exe $Night /inheritance:r /grant:r "${Owner}:(OI)(CI)F" 'SYSTEM:(OI)(CI)F' 'Administrators:(OI)(CI)F' /T /C /Q | Out-Null
if ($LASTEXITCODE -ne 0) { throw "icacls on $Night failed" }
& icacls.exe $Runs /grant "${User}:(OI)(CI)RX" /T /C /Q | Out-Null
if ($LASTEXITCODE -ne 0) { throw "icacls on $Runs failed" }

# 4. Keys (before sshd starts accepting the account).
$keyLines = @(Key-Line $PublicKey $From)
if ($TestPublicKey) { $keyLines += Key-Line $TestPublicKey $TestFrom }
[IO.File]::WriteAllLines($Keys, [string[]]$keyLines)
& icacls.exe $Keys /inheritance:r /grant:r 'SYSTEM:F' 'Administrators:F' /Q | Out-Null
if ($LASTEXITCODE -ne 0) { throw "icacls on $Keys failed" }

# 3. sshd.
$block = @(
    $Begin,
    "AllowUsers $User",
    "Match User $User",
    "    AuthorizedKeysFile __PROGRAMDATA__/ssh/274bot_pull_authorized_keys",
    "    ChrootDirectory $Runs",
    "    ForceCommand internal-sftp -R",
    "    AllowTcpForwarding no",
    "    AllowStreamLocalForwarding no",
    "    AllowAgentForwarding no",
    "    GatewayPorts no",
    "    PermitTunnel no",
    "    PermitTTY no",
    "    X11Forwarding no",
    $End
)
$wanted = [string[]]($outside + $block)
if ((Compare-Object $lines $wanted -SyncWindow 0) -ne $null) { Write-Config $wanted }
"applied: account $User, read-only $Runs, $($keyLines.Count) key(s) in $Keys, sshd block in $Config"
