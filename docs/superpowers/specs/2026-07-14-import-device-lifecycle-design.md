# Import Device Lifecycle Design

## Goal

Make USB/IP import preserve a host device's already-claimed interface state so reset-sensitive devices can serve transfers immediately after attach.

## Problem

The server currently resets and reopens every host device inside `UsbIpServer::occupy`. Import is an ownership transition, not a USB reset request. A physical reset can re-enumerate the device and invalidate the interface claim that transfers depend on. On the Nano Pi, this produced an unclaimed-interface kernel warning and `EIO` when the Linux CH341 driver opened the imported CH340 port.

## Chosen Design

`occupy` will move the existing `UsbDevice` from `available_devices` to `used_devices` without resetting or reopening its physical handle. The device is already opened, its kernel driver detached where needed, and its interfaces claimed by `with_nusb_devices` when it enters the server inventory.

Actual USB/IP reset commands remain the only path that may reset a physical device. This keeps the rule device-agnostic: importing any device does not alter its physical lifecycle.

## Scope

- Remove proactive reset/reopen from the ordinary import path.
- Remove any now-unused reset/reopen helper and imports.
- Add a regression test for the import lifecycle policy.
- Build, deploy, and validate the CH340 on the Pi/Ubuntu lab.

## Non-Goals

- No VID/PID-specific behavior.
- No change to USB/IP protocol reset-command handling.
- No changes to client permissions, device-node ownership, or auto-detach behavior.

## Verification

Unit tests must cover that ordinary import does not request host reset/reopen. On the lab, `usbip list -r` must still export CH340, Ubuntu must attach it, `/dev/ttyUSB0` must bind to `ch341`, and a non-writing `open(O_RDWR | O_NOCTTY | O_NONBLOCK)` must succeed.
