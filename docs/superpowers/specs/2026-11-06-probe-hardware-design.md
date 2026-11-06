# Probe hardware: disk health, USB debug probes and boards, drivers, GPU

Status: decided by the owner 2026-11-06 (option dialogue); built as REQ-PROBE-012.

## Why

Beszel showed nothing about hardware. For an embedded toolchain fleet the questions are:
is a disk wearing out before it fails, which debug probes and boards are attached to which
machine (and did one disappear), which driver holds them, and what GPU a machine has.

## What probes add (`Report.hardware`, one block, published reduced)

| Part | Linux (no root) | macOS (no root) |
|---|---|---|
| Disk health | model, rotational, size from `/sys/block`; NVMe temperature via hwmon; **SMART/wear: see decision** | `diskutil info` SMART status (Verified/Failing); wear and power-on hours via `smartctl --json -a` when installed (works unprivileged on Apple Silicon internal NVMe) |
| USB devices | `/sys/bus/usb/devices/*`: vendor/product id, manufacturer and product strings, speed, bound driver (`*/driver` link) | `ioreg -p IOUSB -l`: vendor/product id, names |
| Debug probes and boards | classified from the USB list by vendor:product id (table below) | same |
| Serial ports | `/dev/serial/by-id` names reduced to the USB device they belong to | `/dev/cu.*` count per USB device |
| GPU | `/sys/class/drm/card*/device/{vendor,device}`, driver; `nvidia-smi --query-gpu=…` when present (utilisation, memory, temperature) | `system_profiler SPDisplaysDataType -json`: model, cores |

Debug probe table (vendor:product → kind), extendable in code: SEGGER J-Link `1366:*`,
ST-LINK `0483:3748/374b/374e/374f/3753/3754`, Raspberry Pi Debug Probe `2e8a:000c`,
CMSIS-DAP by product string, Black Magic Probe `1d50:6018`, FTDI dual/quad `0403:6010/6011/6014`,
NXP LPC-Link2 `1fc9:0090/0143`, TI XDS110 `0451:bef3`, Espressif USB-JTAG `303a:1001`,
Microchip/Atmel EDBG `03eb:2111/2141`, Nordic `1915:*`.

Never published: USB serial numbers (they identify a specific board and person), device paths
beyond the device's own name, user-set device names. Product strings pass the REQ-PROBE-011
whitelist (`clean_label`) or are dropped.

## History and findings

- Series: `disk.wear_pct{dev=}`, `disk.media_errors{dev=}`, `disk.power_on_h{dev=}`,
  `disk.temp_c{dev=}`, `gpu.util_pct{gpu=}`, `gpu.mem_used_pct{gpu=}`, `gpu.temp_c{gpu=}`,
  `usb.probes` (count of attached debug probes).
- Findings: SMART status failing (red); wear at or over 80 % (amber) / 95 % (red); media
  errors increasing (amber); a debug probe that was attached to this machine in the last
  7 days and is gone (amber, named by kind, e.g. "ST-LINK V3 no longer attached"), so a
  hardware-in-the-loop runner losing its probe is visible before its jobs fail.
- Machines card: a chip per attached probe kind ("J-Link", "ST-LINK ×2"); the full history
  gets a "Hardware" group and a "Attached now" list (kind, product, driver).

## Owner's decisions (2026-11-06)

1. **SMART on Linux: no capabilities for the probe; warte ships the privileged part.** First
   answer was "give the probe capabilities"; the owner then decided (relayed by the smithy
   session, same day) that the probe keeps zero capabilities and that the root part must be in
   open-source warte, not a private smithy script: `warte probe privileged`, a root oneshot
   (hourly timer, no network) writes `smart.json`; the probe reads it as untrusted data
   (size cap, strict schema, names and values re-checked) and reports "privileged collector
   not running" when it is missing or stale, never old numbers. Both units are documented in
   `docs/topics/probe-privileged.md`; smithy only deploys them.
2. **A debug probe gone is an amber finding**: attached in the last 7 days, not now.
3. **Macs get smartmontools through smithy** (Homebrew); with it, wear and power-on hours.

## Non-goals

Driver versions beyond the bound driver name; PCI devices other than GPUs; flashing or
talking to probes; anything needing root on macOS.
