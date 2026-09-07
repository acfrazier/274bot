# Native Hyper-V Linux builder preflight

Status: plan only. This document does not create a VM, download an image, change Windows configuration, or request another reboot.

## Target and boundaries

Use the already-enabled Hyper-V host after the completed reboot boundary. The host is Windows 11 IoT Enterprise LTSC 26100 on an Alienware 16 X Aurora AC16251 with an Intel Ultra 9 275HX (24 cores) and 64 GiB RAM. The builder is for Linux Rust builds and tests; it is not a deployment-size recommendation and does not establish a performance result. Keep the memory campaign's native Windows proof, VPS matched builds, and clean measurements separate from builder provisioning.

Recommended bounded builder shape:

- Generation 2, Secure Boot enabled with the `Microsoft UEFI Certificate Authority` template.[2][3]
- 4 vCPU and 8 GiB startup RAM for ordinary builds; an explicit upper bound of 8 vCPU and 16 GiB RAM for a build/test burst. Do not assign the full host.
- 64 GiB dynamically expanding VHDX for the guest and build artifacts, with free space checked before large builds.
- Ubuntu Server 24.04 LTS amd64 cloud image. Canonical publishes a Hyper-V/Azure VHD artifact and SHA256 manifest on the release page.[1]
- No shared password. Use a newly generated public key supplied through cloud-init; keep the private key outside the repository and user-data.

## Immutable image input and verification

Pin one release page snapshot at provisioning time rather than using a moving `daily` image. At the time this plan was written, the release page identified Ubuntu 24.04 LTS release `[20260826]` and listed:

```text
URL: https://cloud-images.ubuntu.com/releases/24.04/release/ubuntu-24.04-server-cloudimg-amd64-azure.vhd.tar.gz
SHA256: 843d243792abb05b50e1a7f5e614e1184d8fc7195c119747cbb3038520258a22
SHA256 manifest: https://cloud-images.ubuntu.com/releases/24.04/release/SHA256SUMS
```

The URL, release date, filename, and digest must be re-read from the official page immediately before root downloads; if any changes, record the new filename and digest together. Do not substitute an unpinned URL or a locally cached image. Verify before extraction/import:

```powershell
$ErrorActionPreference = 'Stop'
$root = 'C:\ProgramData\274bot\hyperv-builder'
New-Item -ItemType Directory -Force $root, "$root\image", "$root\seed", "$root\vm" | Out-Null
Invoke-WebRequest -Uri 'https://cloud-images.ubuntu.com/releases/24.04/release/ubuntu-24.04-server-cloudimg-amd64-azure.vhd.tar.gz' -OutFile "$root\image\ubuntu-24.04-server-cloudimg-amd64-azure.vhd.tar.gz"
$actual = (Get-FileHash "$root\image\ubuntu-24.04-server-cloudimg-amd64-azure.vhd.tar.gz" -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne '843d243792abb05b50e1a7f5e614e1184d8fc7195c119747cbb3038520258a22') { throw "SHA256 mismatch: $actual" }
tar -xzf "$root\image\ubuntu-24.04-server-cloudimg-amd64-azure.vhd.tar.gz" -C "$root\image"
Get-ChildItem "$root\image" -Filter '*.vhd'
```

If the extracted VHD is attached directly, retain it as the verified source and use a separate writable copy for the VM. If Hyper-V requires VHDX for the chosen workflow, convert the verified VHD to a new VHDX and never mutate the verified source:

```powershell
Convert-VHD -Path "$root\image\ubuntu-24.04-server-cloudimg-amd64-azure.vhd" `
  -DestinationPath "$root\vm\ubuntu-builder-os.vhdx" -VHDType Dynamic
Resize-VHD -Path "$root\vm\ubuntu-builder-os.vhdx" -SizeBytes 64GB
```

The resize is applied to the writable copy, not the verified source. Confirm the
logical size with `Get-VHD` before attaching it; the dynamically allocated file
will initially consume less space than its 64 GiB logical capacity.

Record the final source and converted-disk hashes in the provisioning log. A converted VHDX hash is expected to differ from the Canonical tarball digest; only the downloaded tarball is checked against the published digest.

## Cloud-init seed and key-only SSH

Use NoCloud seed data on a small ISO attached only for first boot. NoCloud accepts a volume labeled `CIDATA` containing `user-data` and `meta-data`.[4] The Azure-flavoured image is not safe to treat as NoCloud-ready: inspect its cloud-init configuration before first boot and convert the writable copy's datasource selection offline. A seed ISO alone is insufficient when the image pins `datasource_list: [ Azure ]`.

Before creating the seed ISO or VM, attach the writable VHDX to an approved
offline Linux environment (for example, an already available WSL2 instance using
`wsl --mount <path-to-vhdx> --bare`; do not install WSL or create another VM in
this task), identify the Linux root partition, and mount it read-write. In the
mounted root, inspect `/etc/cloud/cloud.cfg.d/90_dpkg.cfg` and any other
`datasource_list` setting. Change the effective setting to exactly:

```yaml
datasource_list: [ NoCloud ]
```

Remove an Azure-only duplicate setting rather than leaving conflicting files.
Clear the copied image's prior cloud-init state from the mounted root (at
minimum `/var/lib/cloud/`; preserve the package and configuration files), then
unmount and detach the disk cleanly. Record the offline helper and the exact
file diff. Before `Start-VM`, verify from the mounted filesystem that there is
one effective `NoCloud` datasource setting and no effective Azure-only override;
if the root filesystem cannot be mounted or the setting cannot be verified,
stop before VM creation. This is required because cloud-init documents that
datasource discovery is controlled by its datasource list.[4]

`ssh_pwauth: false` and an `ssh_authorized_keys` entry provide key-only SSH
without putting a password in the seed.[5] Generate the key outside the
repository and substitute only the public half below:

```yaml
# user-data (0600 while staging)
#cloud-config
users:
  - default
  - name: builder
    groups: [adm, sudo]
    shell: /bin/bash
    sudo: ["ALL=(ALL) NOPASSWD:ALL"]
    lock_passwd: true
    ssh_authorized_keys:
      - ssh-ed25519 AAAA...REPLACE_WITH_BUILDER_PUBLIC_KEY...
ssh_pwauth: false
disable_root: true
package_update: true
packages:
  - build-essential
  - pkg-config
  - libssl-dev
  - git
  - openssh-server
runcmd:
  - [systemctl, enable, --now, ssh]
```

```text
# meta-data
instance-id: 274bot-hyperv-builder-20260907
local-hostname: 274bot-builder
```

Create the ISO with a reviewed, locally available ISO tool (`oscdimg` from the Windows ADK, or an equivalent approved tool); do not install tools or download dependencies as part of this worker task. The command must make the filesystem label exactly `CIDATA` and include only the two seed files. Keep `user-data` and the generated ISO out of Git. If no approved ISO tool is available, stop before VM creation and have root install/provide that dependency; do not improvise with a shared password.

## VM creation and network safety

Root should first inspect, without changing adapters:

```powershell
Get-VMHost | Select-Object VirtualMachinePath,VirtualHardDiskPath
Get-VMSwitch | Select-Object Name,SwitchType,NetAdapterInterfaceDescription
Get-VM -Name '274bot-builder' -ErrorAction SilentlyContinue
```

Use an existing switch named `Default Switch` if present. If it is absent, stop for an operator decision; do not rebind an external adapter or create a new switch in this bounded setup. The guest IP can be discovered after boot from the guest console or the existing switch's DHCP/neighbor inventory; do not assume a fixed address.

Create an isolated VM and attach the copied OS disk, seed ISO, and existing switch. Exact cmdlets should be run by root after checking that the VM name and paths do not already exist:

```powershell
$vm = '274bot-builder'
New-VM -Name $vm -Generation 2 -MemoryStartupBytes 8GB `
  -VHDPath 'C:\ProgramData\274bot\hyperv-builder\vm\ubuntu-builder-os.vhdx' `
  -Path 'C:\ProgramData\274bot\hyperv-builder\vm' -SwitchName 'Default Switch'
Set-VMProcessor -VMName $vm -Count 4
Set-VMMemory -VMName $vm -DynamicMemoryEnabled $true -StartupBytes 8GB -MinimumBytes 8GB -MaximumBytes 16GB
Set-VMFirmware -VMName $vm -EnableSecureBoot On -SecureBootTemplate 'MicrosoftUEFICertificateAuthority'
Add-VMDvdDrive -VMName $vm -Path 'C:\ProgramData\274bot\hyperv-builder\seed\cidata.iso'
Set-VM -Name $vm -AutomaticStopAction ShutDown
Start-VM $vm
```

If `New-VM` creates a default disk or switch attachment unexpectedly, stop and inspect before proceeding. Keep VM files under `C:\ProgramData\274bot\hyperv-builder`; do not use a personal profile directory or an existing campaign artifact directory.

## First-boot verification and access

From the VM console, wait for cloud-init completion and inspect:

```bash
cloud-init status --wait
cloud-init status --long
lsblk -f
systemctl is-active ssh
sudo sshd -T | grep -E 'passwordauthentication|permitrootlogin'
```

From the Windows host, verify identity and state before using it for builds:

```powershell
Get-VM -Name '274bot-builder' | Select-Object State,Status,Generation,ProcessorCount,MemoryStartup
Get-VMNetworkAdapter -VMName '274bot-builder' | Select-Object MacAddress,Status,SwitchName,IPAddresses
ssh -o PasswordAuthentication=no -o IdentitiesOnly=yes -i '<PRIVATE_KEY_PATH>' builder@<DISCOVERED_IP> 'id; uname -a; systemd-detect-virt; cloud-init status --wait'
```

The SSH test must fail if password-only authentication is attempted. Confirm the guest is Ubuntu 24.04 amd64, the expected public-key fingerprint is the one generated for this VM, and `systemd-detect-virt` reports `microsoft`/Hyper-V before installing project dependencies. Then install the exact Rust/toolchain dependencies required by the campaign and capture versions in the build receipt; toolchain installation is separate from this preflight.

## Recovery, teardown, and reboot boundary

Before the first dependency install, create one named checkpoint only after the verified first boot and SSH test:

```powershell
Checkpoint-VM -Name '274bot-builder' -SnapshotName 'pre-campaign-toolchain'
Get-VMSnapshot -VMName '274bot-builder'
```

For a failed seed, bad network, or failed boot, stop the VM, inspect Hyper-V event logs and `cloud-init` logs from the console, and restore/remove the VM rather than editing the known-good image in place. Preserve the verified source and seed inputs until the first successful SSH verification. Teardown is explicit: `Stop-VM`, then remove the VM and its isolated `C:\ProgramData\274bot\hyperv-builder\vm` directory only after campaign artifacts are copied and hashes recorded.

No additional host reboot is requested by this plan. Hyper-V enablement and the required restart were already completed; any later reboot must be separately coordinated because it interrupts concurrent native proof work.

## Sources

[1] https://cloud-images.ubuntu.com/releases/24.04/release
[2] https://documentation.ubuntu.com/server/how-to/virtualisation/ubuntu-on-hyper-v/index.html
[3] https://learn.microsoft.com/en-us/windows-server/virtualization/hyper-v/Supported-Ubuntu-virtual-machines-on-Hyper-V
[4] https://cloudinit.readthedocs.io/en/stable/reference/datasources/nocloud.html
[5] https://cloudinit.readthedocs.io/en/stable/topics/examples.html
[6] https://cloudinit.readthedocs.io/en/stable/reference/base_config_reference.html
