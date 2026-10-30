# Change Log

## [1.0.6] - 2026-10-30

- Include the upstream fix for 16-bit `set_int` encoding, which now sends the
  correct two bytes for values from -32768 through 32767.

- Discover mixers through all active IPv4 interfaces that have broadcast
  addresses, excluding loopback and point-to-point interfaces.
- Bind discovery sockets to their interfaces on Apple platforms, fixing failed
  discovery when a VPN captures the default broadcast route.
- Merge duplicate replies by IP address and serial number and continue scanning
  if an individual interface fails.
- Use a shared five-second scan deadline; return early for `scan(true)`.
- Ignore malformed discovery replies, including invalid IPv4 addresses.
- Preserve the existing Rust and C discovery APIs.
- Add regression tests for parsing, duplicate replies, early return and timeout;
  correct the Rust connection examples so documentation tests compile.

Validation: three unit tests and two documentation tests pass. A rebuilt
Studio161 automatically discovered and connected to a WING Compact on macOS
with a Tailscale exit node enabled and LAN access allowed, without an explicit
mixer IP argument. Other operating systems have not been tested with hardware
for this change.

## [1.0.4] - 2025-03-04

- removed eframe dependency from libwing

## [1.0.3] - 2025-03-03

- Close read socket when dropping WingConsole

## [1.0.2] - 2025-03-03

- Made public fields of WingConsoleHandle and ResponseHandle

## [1.0.1] - 2025-03-03

- Exposed ffi WingConsoleHandle and ResponseHandle so you can write async wrappers around read()

## [1.0.0] - 2025-03-03

- Calls are thread safe now
- NodeData no longer contains a channel ID (the first parameter), which was always the same before.
- Updates docs

## [unreleased] - 2025-02-17
- Added meters API
