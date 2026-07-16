# Host Configuration Descriptor Preservation

## Goal

Make imported host USB devices expose their original configuration descriptors so
Linux class drivers, including `cdc_acm`, receive all class-specific and
vendor-specific descriptors.

## Design

On Linux, `UsbIpServer::new_from_host_with_filter` will read the device's
`/sys/bus/usb/devices/<bus-id>/descriptors` file while the device is prepared
for export. It will extract and retain each configuration descriptor exactly as
reported by the host. A `GET_DESCRIPTOR(Configuration)` request will return the
retained descriptor, truncated only to the request's `wLength`.

If the sysfs descriptor cannot be read or parsed, the server will retain the
current synthesized-descriptor behavior. This preserves support for existing
test devices and non-Linux builds.

## Scope and Verification

The change does not alter USB/IP transport, endpoint forwarding, attach/detach,
or control-transfer handling. Unit tests cover raw descriptor extraction,
selection, truncation, and synthesized fallback. The Linux E2E check verifies
that Espressif `303a:1001` binds `cdc_acm` and creates `/dev/ttyACM*`; existing
CP2102 and CH340 exports must still enumerate.
