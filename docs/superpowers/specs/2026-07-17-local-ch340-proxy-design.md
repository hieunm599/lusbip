# Local CH340 USB/IP Proxy Design

## Goal

Make forced ESP32-P4 flash over the NanoPi CH340 substantially faster without changing the ESP project, its `/dev/ttyUSB0` consumer, or the NanoPi's physical USB wiring.

## Context

The CH340 exposes 32-byte full-speed bulk endpoints. The Linux `ch341` client serializes small write URBs when it talks directly to the NanoPi over USB/IP. A LAN round trip per 32-byte URB limits a 921600-baud flash to roughly 5--8 KiB/s. Server-side buffering alone cannot remove that round trip because the kernel client waits for each remote USB/IP completion.

## Chosen Architecture

Add a localhost-only USB/IP accelerating proxy to the `lusbip` client. `lusbip attach` gains an explicit `--accelerate` mode:

1. The client starts a proxy listener on `127.0.0.1` using a deterministic, per-remote/bus-id port and opens one USB/IP import session to the NanoPi server.
2. `lusbip` asks the existing Linux `usbip` tool to attach the virtual host controller to the localhost proxy, not directly to NanoPi.
3. The kernel continues to bind the stock `ch341` driver and creates `/dev/ttyUSB0` normally.
4. The proxy replies immediately to small local CH340 bulk-OUT URBs after copying them to a bounded local buffer. It combines contiguous bytes into remote USB/IP bulk-OUT submits up to 4 KiB.
5. Before a local bulk-IN or EP0 control request is forwarded, the proxy flushes all buffered OUT bytes to the NanoPi and waits for its USB/IP completion. This preserves serial command, response, reset, and baud-change ordering.

The critical difference from the rejected server-only timer experiment is location: local completions have loopback latency, so a 4 KiB proxy buffer fills before its short idle flush. The NanoPi still receives normal, larger USB/IP transfers and writes legal 32-byte physical USB packets to the CH340.

## Protocol Responsibilities

`src/usbip_proxy` owns two independent USB/IP directions:

- **Local server side:** accepts `OP_REQ_DEVLIST`, `OP_REQ_IMPORT`, `USBIP_CMD_SUBMIT`, and `USBIP_CMD_UNLINK` from `vhci-hcd`; exposes one virtual device copied from the remote import response.
- **Remote client side:** opens a TCP connection to NanoPi, sends `OP_REQ_IMPORT`, and forwards non-batched commands. It matches remote `USBIP_RET_*` replies to the forwarded request before responding to local requests that need a physical result.

Only bulk-OUT requests for explicitly recognized serial endpoints are batched. Control, interrupt, isochronous, unknown endpoints, and all devices other than the selected CH340 are forwarded one-for-one. The initial implementation recognizes vendor `1a86:7523` and endpoint `0x02`; it must refuse `--accelerate` with a clear error for anything else.

## Ordering And Failure Behavior

- A local bulk-OUT acknowledgement means bytes are retained in the proxy's bounded buffer, not that they are already on the physical wire.
- A local bulk-IN, EP0 control request, threshold fill, disconnect, or a short idle interval flushes that buffer before the next ordered request is forwarded.
- A remote flush failure poisons the proxy session. Subsequent local requests receive USB/IP failures and the proxy closes rather than falsely reporting a successful serial write.
- Local unlink cancels a not-yet-forwarded local IN request. Acknowledged buffered OUT bytes are intentionally not selectively removed, matching the existing server behavior.
- Proxy exit releases the remote import session and removes its runtime socket/pid files. It never detaches arbitrary VHCI ports; normal `lusbip` stale-target matching remains in charge.

## CLI And Lifecycle

`lusbip attach --accelerate --remote 10.10.61.72 --bus-id 5-1` is the supported accelerated workflow. It verifies the target is a CH340, starts or reuses the localhost proxy, auto-detaches only an identified stale target port, then invokes `usbip attach -r 127.0.0.1 -b <virtual-bus-id>`. The existing direct attach mode remains unchanged and is the fallback for CP2102, ACM, and general USB devices.

`lusbip status` reports an accelerated attachment as local proxy-backed and identifies the original remote host/bus id. `lusbip detach` stops only the matching proxy after its VHCI port is detached.

## Testing And Acceptance

- Unit tests cover endpoint eligibility, bounded batching, barrier ordering, response mapping, poisoned-session behavior, and CLI validation.
- Existing direct-attach tests continue to pass unchanged.
- E2E uses NanoPi `pi@10.10.61.72`, Ubuntu `hieunm@10.10.60.208`, CH340 `1a86:7523` at bus `5-1`.
- `espflash monitor` must connect and print a boot log after an accelerated attach.
- A forced `espflash flash --no-skip --baud 921600` must complete and verify successfully. Its elapsed time is compared against the current direct baseline of 8m46s; success is a materially lower elapsed time with no `MemEnd` or communication timeout.

## Non-goals

- Modifying the Linux `ch341` or `vhci-hcd` kernel driver.
- Advertising an invalid endpoint packet size or device speed.
- Accelerating arbitrary USB devices or protocol classes in this change.
- Changing direct USB/IP behavior for CP2102, ACM, storage, HID, or unrelated attachments.
