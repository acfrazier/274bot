# Generic Ubuntu Hyper-V image preparation receipt

Date: 2026-09-07
Status: image prepared and locally verified; VM creation, boot, cloud-init execution, and SSH were not attempted.
Scope: local macOS preparation only. No Windows, Hyper-V, SSH, game/live, or Rust actions were performed.

## Deliverables

- Immutable source (mode 0444): `diagnostics/hyperv-image-preparation-20260907/source/ubuntu-24.04-server-cloudimg-amd64.img`
- Immutable VHDX (mode 0444): `diagnostics/hyperv-image-preparation-20260907/output/ubuntu-24.04-server-cloudimg-amd64-generation2-64G.vhdx`
- Source release page, SHA256 manifest, inspection and conversion logs are retained under `diagnostics/hyperv-image-preparation-20260907/`.

The VHDX is a dynamic VHDX with a 64 GiB virtual capacity and is intended for a Hyper-V Generation 2 VM. It is not evidence that a VM boots.

## Official source pin

- Release page: https://cloud-images.ubuntu.com/releases/noble/release/
- Release page title: `Ubuntu 24.04 LTS (Noble Numbat) release [20260826]`
- Listed artifact: `ubuntu-24.04-server-cloudimg-amd64.img`
- Listed timestamp: `2026-08-26 20:31`
- Artifact URL: https://cloud-images.ubuntu.com/releases/noble/release/ubuntu-24.04-server-cloudimg-amd64.img
- Manifest URL: https://cloud-images.ubuntu.com/releases/noble/release/SHA256SUMS
- Published SHA256: `d0fe84bb5f80853425fa6be28e2c106f30104c3cfe8611933f2e65c9b63f0e30`
- Downloaded source size: 624,829,952 bytes (596 MiB virtual image)
- Downloaded source SHA256: `d0fe84bb5f80853425fa6be28e2c106f30104c3cfe8611933f2e65c9b63f0e30` (matches manifest)

The source image was downloaded once from the pinned official URL and was not modified. The manifest and fetched release page are retained as evidence.

## Offline image evidence

Inspection used a temporary official `ubuntu:24.04` Linux ARM64 container on the native ARM64 Mac (`uname -m`: `aarch64`). The container was temporary and used read-only source mounts; no existing campaign container, volume, image, or build image was used.

The source GPT layout was inspected after a read-only QCOW2-to-raw inspection conversion:

- Partition 1: sectors 2,099,200–7,339,998, Linux filesystem, 2.5 GiB
- Partition 14: sectors 2,048–10,239, BIOS boot, 4 MiB
- Partition 15: sectors 10,240–227,327, EFI System, 106 MiB
- Partition 16: sectors 227,328–2,097,152, Linux extended boot, 913 MiB

EFI evidence from the actual FAT EFI partition includes:

- `EFI/ubuntu/grubx64.efi`
- `EFI/ubuntu/shimx64.efi`
- `EFI/ubuntu/mmx64.efi`
- `EFI/ubuntu/grub.cfg`
- `EFI/ubuntu/BOOTX64.CSV` was present in the inspected EFI tree

The actual root filesystem was extracted read-only to a temporary inspection file and inspected with `debugfs`. Its cloud-init configuration contains:

`/etc/cloud/cloud.cfg.d/90_dpkg.cfg`

```yaml
datasource_list: [ NoCloud, ConfigDrive, OpenNebula, DigitalOcean, Azure, AltCloud, VMware, OVF, MAAS, GCE, OpenStack, CloudSigma, SmartOS, Bigstep, Scaleway, AliYun, Ec2, CloudStack, Hetzner, IBMCloud, Oracle, Exoscale, RbxCloud, UpCloud, Vultr, LXD, NWCS, Akamai, WSL, CloudCIX, None ]
```

No offline image mutation was necessary: NoCloud is already the first effective datasource and the image is not Azure-only. No cloud-init state, userdata, password, SSH key, or other credentials were injected.

## VHDX conversion evidence

The source was converted from explicit QCOW2 to VHDX with ARM64 `qemu-img` 8.2.2 (`Ubuntu package 1:8.2.2+ds-0ubuntu1.18`). Because this qemu build reports that VHDX does not support the post-conversion `qemu-img resize` operation, the writable output was first created as a dynamic 64 GiB VHDX and the source was then converted into that pre-sized target. The source remained read-only throughout.

Final qemu-img verification:

```text
file format: vhdx
virtual size: 64 GiB (68719476736 bytes)
disk size: 3.51 GiB
file length: 3.51 GiB (3766484992 bytes)
SHA256: a77e2980c3d35ebc27f84a892b005ea7500b5d4eb68b6c0afdbb93303dc72e2f
```

The final VHDX was chmodded to mode 0444 and re-opened read-only by a fresh ARM64 qemu-img verification container. All temporary containers created for this task were removed after the checks. Docker Desktop's pre-existing image/cache was not pruned or otherwise cleaned.

## Root handoff

Root may copy the VHDX to the Windows Hyper-V staging directory and create the Generation 2 VM, then attach a separately created `CIDATA` seed ISO. Root must still verify Windows-side VHDX transfer hash and perform all VM/boot/cloud-init/SSH checks. This receipt intentionally makes no claim about those later operations.

Evidence files:

- `diagnostics/hyperv-image-preparation-20260907/source/release-page.html`
- `diagnostics/hyperv-image-preparation-20260907/source/SHA256SUMS`
- `diagnostics/hyperv-image-preparation-20260907/logs/convert.log`
- `diagnostics/hyperv-image-preparation-20260907/logs/resize.log`
- `diagnostics/hyperv-image-preparation-20260907/logs/gpt.log`
- `diagnostics/hyperv-image-preparation-20260907/logs/datasource-inspection.log`
- `diagnostics/hyperv-image-preparation-20260907/logs/filesystem-inspection.log`
- `diagnostics/hyperv-image-preparation-20260907/logs/tool-install.log`
- `diagnostics/hyperv-image-preparation-20260907/logs/final-verify.log`
