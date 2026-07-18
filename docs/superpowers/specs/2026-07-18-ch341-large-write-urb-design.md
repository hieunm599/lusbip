# CH341 Large Write URB Experiment Design

## Goal

Determine whether increasing the Linux `ch341` driver's bulk-OUT buffer from
the endpoint default of 32 bytes to 256 bytes removes the USB/IP round-trip
throughput limit while preserving ESP32-P4 flashing and serial control.

## Scope

This is a host-specific experiment for Ubuntu `7.0.0-27-generic` and CH340
`1a86:7523`. It does not change the `lusbip` USB/IP protocol, NanoPi server,
ESP firmware project, CP210x behavior, or the device's USB descriptors.

The experiment builds an out-of-tree `ch341.ko` from the upstream Linux v7.0
driver with one functional change:

```c
static struct usb_serial_driver ch341_device = {
    .driver = {
        .name = "ch341-uart",
    },
    .id_table          = id_table,
    .num_ports         = 1,
    .bulk_out_size     = 256,
    // Existing callbacks remain unchanged.
};
```

Linux USB serial still keeps two write URBs. The USB core splits each 256-byte
URB into legal 32-byte packets for the physical CH340 endpoint, while USB/IP
crosses the LAN once per URB rather than once per 32-byte buffer.

## Build And Compatibility

- Build only against `/lib/modules/$(uname -r)/build`.
- Require `uname -r` to equal `7.0.0-27-generic` for the first experiment.
- Require the built module's vermagic to match the running kernel.
- Do not install into `/lib/modules`; load the experiment with `insmod` so a
  reboot or explicit rollback restores the packaged module.
- Secure Boot must be disabled because the experimental module is unsigned.

## Runtime Safety And Rollback

Before changing modules:

1. Confirm no process has `/dev/ttyUSB0` open.
2. Identify and detach only the VHCI port containing CH340 `1a86:7523`.
3. Run `modprobe -r ch341`, then `insmod` the experimental module.
4. Reattach only NanoPi bus `5-1` and confirm `ch341` creates `/dev/ttyUSB0`.

Rollback detaches only the CH340 VHCI port, unloads the experimental module,
and runs `modprobe ch341`. It never replaces or deletes the packaged module.

## Verification

Verification proceeds in increasing-risk order:

1. `modinfo` confirms module name and matching vermagic.
2. Direct USB/IP attach enumerates CH340 and creates `/dev/ttyUSB0`.
3. `espflash board-info` or monitor exercises baud and DTR/RTS control without
   modifying flash.
4. A forced application flash at 921600 baud writes and verifies all data.

The baseline is 2,884,000 bytes (1,359,885 compressed) in 522.6 seconds. The
experiment passes only if the write verifies successfully and elapsed time is
materially below 522.6 seconds. Kernel disconnects, control timeouts, corrupted
flash, or no meaningful improvement are failures and trigger rollback.

## Evidence

Record the running kernel, stock and experimental `modinfo`, CH340 sysfs
endpoints, `usbip port`, relevant kernel log lines, exact flash command, elapsed
time, throughput, and verification result. Do not claim success from compilation
or enumeration alone.

