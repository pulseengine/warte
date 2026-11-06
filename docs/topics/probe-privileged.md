# Privileged collector (`warte probe privileged`)

Some readings need root: on Linux, SMART disk health (wear, media errors, power-on hours,
temperature). The probe that sends reports never runs as root and has no capabilities.
Instead a second, small part of warte runs as root, does only that collection, and writes the
result to a file the probe reads (REQ-PROBE-012).

- `warte probe privileged [--out DIR]` reads SMART for every whole disk
  (`smartctl --json -a -n standby`, so a sleeping disk is not woken) and writes
  `DIR/smart.json` (default `/var/lib/warte-privileged`, mode 0644, the directory 0755).
  It opens no network connection. It writes reduced data only: device name, model, SMART
  passed, wear, media errors, power-on hours, temperature; no serial numbers.
- The probe (`warte probe send`) reads that file as untrusted data: at most 256 KiB, the exact
  schema `warte.privileged.v1` (unknown fields refused), device and model names through the
  probe's whitelist, values within range. Set `WARTE_PRIVILEGED_DIR` if you change the
  directory.
- When the file is missing, the probe reports `privileged: missing` (shown as "SMART: collector
  not running" on the machine's card). When it is older than 2 h 10 min or dated in the future,
  `stale`, plus an amber finding. A refused file is `invalid`, also amber. In none of these
  cases does it report old numbers.

On macOS no collector is needed: `smartctl` reads the internal SSD as an ordinary user, and the
probe runs it itself, at most hourly, when smartmontools is installed.

## systemd units (Linux)

Install smartmontools, then:

```ini
# /etc/systemd/system/warte-privileged.service
[Unit]
Description=warte privileged collector (SMART for the probe)

[Service]
Type=oneshot
ExecStart=/usr/local/bin/warte probe privileged --out /var/lib/warte-privileged
# root, for SMART only; nothing else
StateDirectory=warte-privileged
StateDirectoryMode=0755
ProtectSystem=strict
ReadWritePaths=/var/lib/warte-privileged
ProtectHome=yes
PrivateTmp=yes
PrivateNetwork=yes
RestrictAddressFamilies=AF_UNIX
IPAddressDeny=any
NoNewPrivileges=yes
ProtectKernelModules=yes
ProtectControlGroups=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
RestrictRealtime=yes
SystemCallArchitectures=native
```

```ini
# /etc/systemd/system/warte-privileged.timer
[Unit]
Description=Run the warte privileged collector hourly

[Timer]
OnBootSec=2min
OnUnitActiveSec=1h
RandomizedDelaySec=5min

[Install]
WantedBy=timers.target
```

`systemctl enable --now warte-privileged.timer`. The probe's own unit stays as it is: an
unprivileged user, no `AmbientCapabilities`, read access to `/var/lib/warte-privileged` (it is
world-readable). `PrivateDevices=` must stay off for the collector, since it opens the disks.
