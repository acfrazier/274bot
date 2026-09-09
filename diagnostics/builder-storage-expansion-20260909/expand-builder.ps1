$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$vm=Get-VM -Name '274bot-builder'
if ($vm.State -ne 'Off') { throw 'Builder must be shut down cleanly' }
$drives=@(Get-VMHardDiskDrive -VMName $vm.Name)
if ($drives.Count -ne 1) { throw 'Unexpected disk count' }
$drive=$drives[0]
$source='C:\ProgramData\274bot\hyperv-builder\vm\ubuntu-builder-os_701763EC-054C-4283-A727-BB7BC8AD071C.avhdx'
$dest='C:\ProgramData\274bot\hyperv-builder\vm\ubuntu-builder-os-expanded-128.vhdx'
$out='C:\ProgramData\274bot\hyperv-builder\storage-expansion-20260909'
if ($drive.Path -ne $source -or (Test-Path $dest) -or (Test-Path $out)) { throw 'Unexpected source or existing output' }
$chain=@(); $current=$source
while ($current) {
 $v=Get-VHD -Path $current; $f=Get-Item -LiteralPath $current
 $chain += [pscustomobject]@{Path=$current;Size=$v.Size;FileSize=$f.Length;WriteUtc=$f.LastWriteTimeUtc.Ticks;ParentPath=$v.ParentPath}
 $current=$v.ParentPath
}
if ($chain[0].Size -ne 64GB) { throw 'Expected 64GiB source' }
New-Item -ItemType Directory -Path $out|Out-Null
$before=[pscustomobject]@{Chain=$chain;Snapshots=@(Get-VMSnapshot -VMName $vm.Name|Select-Object Name,Id);Source=$source;Destination=$dest;SizeBytes=128GB}
$before|ConvertTo-Json -Depth 6|Set-Content -Encoding UTF8 "$out\before.json"
Write-Output 'START independent disk conversion'
Convert-VHD -Path $source -DestinationPath $dest -VHDType Dynamic
$v=Get-VHD -Path $dest
if ($v.ParentPath -or $v.Size -ne 64GB) { throw 'Independent copy verification failed' }
Resize-VHD -Path $dest -SizeBytes 128GB
$v=Get-VHD -Path $dest
if ($v.ParentPath -or $v.Size -ne 128GB) { throw 'Expansion verification failed' }
foreach ($row in $chain) {
 $f=Get-Item -LiteralPath $row.Path
 if ($f.Length -ne $row.FileSize -or $f.LastWriteTimeUtc.Ticks -ne $row.WriteUtc) { throw 'Original disk chain changed' }
}
Set-VMHardDiskDrive -VMHardDiskDrive $drive -Path $dest
Start-VM -Name $vm.Name
$after=[pscustomobject]@{VM=Get-VM -Name $vm.Name|Select-Object Name,State;Disk=Get-VHD -Path $dest|Select-Object Path,Size,FileSize,ParentPath;Snapshots=@(Get-VMSnapshot -VMName $vm.Name|Select-Object Name,Id);OriginalChainMetadataUnchanged=$true}
$after|ConvertTo-Json -Depth 6|Set-Content -Encoding UTF8 "$out\after.json"
$after|ConvertTo-Json -Depth 6
