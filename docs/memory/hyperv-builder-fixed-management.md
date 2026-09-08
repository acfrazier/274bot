# Builder fixed management network

Configured 2026-09-08 at the operator's request to avoid rediscovering the builder IP after restarts.

The existing VM `274bot-builder` (ID `34954ade-f213-45ef-8957-dfafdbaa53fc`) now has a dedicated internal switch `274bot-BuilderManagement` (ID `cc4fa655-a25b-4c8c-b4f8-fea65da36faf`). Windows has `192.168.247.1/30`; the guest management adapter has static `192.168.247.2/30`, MAC `00:15:5d:00:cd:02`, interface `mgmt0`. No default gateway or DNS is assigned to this interface. Guest configuration is `/etc/netplan/60-builder-management.yaml`, matched by MAC and mode0600.

The original Default Switch adapter remains responsible for DHCP, default route and DNS/internet downloads. Its address may change without affecting SSH. No additional NAT, bridge, port forward or firewall exception was added. Windows Wi-Fi/default route remained `10.0.0.205` via `10.0.0.1`; no host reboot or host network reset occurred. Existing Default Switch was retained.

Mac `~/.ssh/config` alias `274bot-builder` now targets `192.168.247.2` via the existing Windows SSH proxy. Existing strict host-key checking, pinned guest `HostKeyAlias 172.30.253.116`, identity keys and known-host files are unchanged. This removes guest DHCP rediscovery; normal host-key and readiness checks still apply. The proxy still depends on the Windows host address.

Verified SSH, routing, DNS and HTTPS Rust download endpoint before and after a guest-only reboot. Boot identity changed from `e3dcfd1e-843b-4244-9e1e-034106d7d229` to `d63ba1f3-9677-4e61-9d4c-f14a8cf502ac`. DHCP changed from `172.23.88.107` to `172.23.92.174`; management remained `192.168.247.2` and HTTPS returned200. Windows reboot persistence is configured but not tested by rebooting the operator's host.

During initial guest configuration, networkctl reload renewed the old DHCP lease and interrupted that SSH session. Root recovered the new address by the existing verified VM MAC/SSH key, renamed only the new guest interface, and verified the final fixed connection. No application build or profiling run was active on this VM.

Evidence: `diagnostics/builder-fixed-management-20260908/` contains Windows setup and pre/post reboot receipts. Historical builder receipts retain their original addresses. Future work uses the SSH alias.

Rollback if required: while both guest adapters are present, verify the current Default Switch DHCP path, switch the Mac alias to that verified address, remove only `/etc/netplan/60-builder-management.yaml`, then remove only the VM adapter `Builder Management` and switch `274bot-BuilderManagement`. Do not remove the original adapter or Default Switch.
