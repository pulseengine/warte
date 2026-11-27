# Bench devices

Since v1.6.57 the probe reports which registered bench devices (debug probes,
boards, bus interfaces) are attached to its host (REQ-PROBE-019).

## The registry

The probe reads the bench device registry, the same file device claims use:
`WARTE_BENCH_REGISTRY` if set, else `$HOME/.config/pulseengine/bench-devices.yaml`.
No file means the host is not a bench host, and nothing is reported.

```yaml
devices:
  stlink-v3:
    what: a debug probe
    serial: 0123456789ABCDEF      # matched against USB serials
  esp-jtag:
    usb: 303a:1001                # matched against vendor:product ids
    missing-alert-minutes: 30     # default 15; 0 never alerts
  can0:
    iface: can0                   # matched against network interfaces
  disk-io:
    what: a pseudo-device         # no serial, usb or iface: skipped
```

When an entry has more than one key, `serial` decides first, then `usb`, then
`iface`. An interface counts as present unless its operational state is `down`.

## What leaves the host

Only each device's name, present or missing, which key matched, since when it
is missing and its threshold. Serials and USB ids stay on the host. Names
outside `[a-z0-9][a-z0-9._-]*` (at most 48 bytes) are skipped and counted, and
warte checks them again before showing them.

Presence comes from sysfs only (`/sys/bus/usb/devices`, `/sys/class/net`). The
probe never opens a tty, a CAN socket or any device node, because those need a
claim. Without sysfs (macOS) the host shows "bench: not readable here". A registry
larger than 64 KiB, or one that is not YAML, shows as refused.

## On the Machines view and in alerts

Each machine card lists its bench devices, missing first. A device missing for
its threshold or longer is a red finding, which notifies like other machine
outages. One missing for less, or with `missing-alert-minutes: 0`, is amber.
Both clear when the device returns.
