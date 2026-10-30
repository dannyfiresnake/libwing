# libwing Architecture

## High-Level Design

```
┌──────────────────────────────────────────────┐
│            Application Code                  │
│  (Rust crate users, C FFI consumers, tools)  │
└───────────────────┬──────────────────────────┘
                    │
        ┌───────────┼───────────┐
        │           │           │
   Rust API    C FFI API    CLI Tools
  (lib.rs)    (ffi.rs)    (tools/*.rs)
        │           │           │
        └───────────┼───────────┘
                    │
            ┌───────┴────────┐
            │  WingConsole   │  (console.rs)
            │                │
            │  ┌──────────┐  │
            │  │ Protocol │  │  Binary encode/decode
            │  │ Engine   │  │  Escape handling
            │  └──────────┘  │  Channel state machine
            │                │
            │  ┌──────────┐  │
            │  │ Property │  │  name_to_id / id_to_defs
            │  │ Map      │  │  78k entries (propmap.rs)
            │  └──────────┘  │
            └───────┬────────┘
                    │
          ┌─────────┴─────────┐
          │                   │
     TCP:2222            UDP:2222
     (control)          (discovery, meters)
          │                   │
          └─────────┬─────────┘
                    │
          ┌─────────┴─────────┐
          │  Behringer Wing   │
          │  Hardware         │
          └───────────────────┘
```

## Module Responsibilities

### `lib.rs` — Public Surface

The crate root. Defines module declarations and re-exports the public API:
- `WingConsole`, `DiscoveryInfo`, `Meter` (from `console`)
- `WingNodeDef`, `WingNodeData`, `NodeType`, `NodeUnit` (from `node`)
- `WingConsoleHandle`, `ResponseHandle` (from `ffi`)
- `WingResponse` enum — the return type of `read()`
- `Error` enum via `thiserror`

### `console.rs` — Connection & Protocol Engine (~665 lines)

The core module. Everything network-related lives here.

#### Struct Layout

```rust
WingConsole {
    rsock: Arc<Mutex<TcpStream>>,      // read socket
    wsock: Arc<Mutex<TcpStream>>,      // write socket (cloned from same TCP stream)
    main:  Arc<Mutex<_WingConsoleMain>>,   // data read state
    mtrs:  Arc<Mutex<_WingConsoleMeters>>, // meter state
}
```

Four independent mutexes allow concurrent reads, writes, and meter operations without blocking each other.

#### Protocol State Machine (`decode_next`)

The core decode loop in `decode_next()` processes the binary stream byte-by-byte:

1. Read bytes from `rx_buf` (refilled from TCP when empty)
2. Track escape state (`rx_esc`): `0xdf` starts an escape sequence
3. Within an escape: `0xde` = literal `0xdf`, `0xd0`–`0xdd` = set channel (command context), anything else = new message boundary
4. Track `rx_current_channel` — the command byte that determines how to interpret data
5. `rx_has_in_pipe` handles the case where a new message boundary is detected mid-decode

#### Command Dispatch (`read`)

`read()` calls `decode_next()` repeatedly and dispatches on the command byte:

| Byte Range | Meaning |
|------------|---------|
| 0x00–0x3f | Inline small integer value → `NodeData(id, i32)` |
| 0x80–0xbf | Short string (length = byte - 0x80 + 1) → `NodeData(id, string)` |
| 0xc0–0xcf | Another short string encoding |
| 0xd0 | Empty string → `NodeData(id, "")` |
| 0xd1 | Long string (length in next byte) |
| 0xd3 | 16-bit signed integer |
| 0xd4 | 32-bit signed integer |
| 0xd5, 0xd6 | 32-bit float |
| 0xd7 | Set current node ID (next 4 bytes) |
| 0xde | Request end marker → `RequestEnd` |
| 0xdf | Node definition (length prefix, then raw bytes → `NodeDef`) |

#### Writing

`format_id()` encodes a 32-bit property ID into the wire format, escaping any `0xdf` bytes with `0xde`. The `set_*` methods build on this:

- `set_int`: inline (0x00–0x3f), 16-bit (0xd3), or 32-bit (0xd4) depending on value
- `set_float`: always 0xd5 + 4 bytes big-endian
- `set_string`: 0xd0 (empty), inline length (≤64 chars), or 0xd1 + length byte

#### Keep-Alive

- Data: sends `[0xdf, 0xd1]` every 7 seconds (same as handshake)
- Meters: resends the meter subscription packet every 3 seconds, one per active meter ID
- Both are called automatically during `read()` / `read_meters()`, but can be called manually if not reading

#### Meters

Meters use a **separate UDP socket** (ephemeral port chosen by OS). The subscription request includes the port number so the Wing knows where to send meter data. `read_meters()` returns `(meter_id, Vec<i16>)` — the ID correlates back to the `request_meter()` call.

#### Discovery

`scan()` delegates to `src/discovery.rs`, which sends `"WING?"` over UDP to
each active IPv4 interface's subnet broadcast address on port 2222. Loopback
and point-to-point interfaces are excluded. Each probe uses a separate
nonblocking socket bound to the local address; Apple platforms also use
`IP_BOUND_IF` to avoid VPN default broadcast routes. Interface failures are
isolated. Responses are CSV: `WING,ip,name,model,serial,firmware`, validated and
deduplicated by IP address and serial number. All sockets share a five-second
deadline, with early return when `stop_on_first` is true. See `Discovery.md`.

#### Drop

`WingConsole::drop` shuts down both read and write sides of the TCP connection.

### `node.rs` — Data Model (~545 lines)

Defines the property schema and value structures.

**`WingNodeDef`** — a property's full definition, deserialized from raw bytes:
- IDs: `id`, `parent_id`, `index`
- Names: `name`, `long_name`
- Type info: `node_type`, `unit`, `read_only`
- Type-specific: `min_float/max_float`, `min_int/max_int`, `steps`, `max_string_len`
- Enum values: `string_enum` (Vec of item/long_item), `float_enum` (Vec of value/long_item)
- `raw`: the original bytes, preserved for propmap generation

**`WingNodeData`** — holds a single property value as one of string, f32, or i32.

**`NodeType`** enum (8 variants): Node, LinearFloat, LogarithmicFloat, FaderLevel, Integer, StringEnum, FloatEnum, String.

**`NodeUnit`** enum (8 variants): None, Db, Percent, Milliseconds, Hertz, Meters, Seconds, Octaves.

Both enums are `#[repr(C)]` for FFI compatibility.

`from_bytes()` parses the binary wire format: parent_id (4B) → id (4B) → index (2B) → name (len-prefixed) → long_name (len-prefixed) → flags (2B, encodes type/unit/readonly) → type-specific fields.

### `propmap.rs` — Property Mappings (auto-generated)

A single `lazy_static` block that deserializes a large byte string literal into `HashMap<String, WingNodeDef>`. The byte string is a packed format: for each entry, a flag byte, then length-prefixed name, then length-prefixed raw node def bytes.

`console.rs` also builds a reverse map (`ID_TO_NAME: HashMap<i32, Vec<String>>`) from this data at startup, since multiple names can map to the same ID (due to dynamic properties).

### `ffi.rs` — C FFI Bindings (~644 lines)

Wraps every public Rust API in `extern "C"` functions with opaque pointer types. Pattern:
- Rust structs wrapped in `Box` and returned as raw pointers
- String params as `*const c_char`, returned strings as `*mut c_char` (caller must free)
- Every allocation has a matching `_free()` function
- Documented in `libwing.h`

## Threading Model

```
Thread 1: loop { wing.read() }           // blocks on rsock + main mutex
Thread 2: loop { wing.read_meters() }    // blocks on meter UDP socket + mtrs mutex
Thread 3: wing.set_float(id, val)        // takes wsock mutex briefly
Thread 4: loop { wing.keep_alive() }     // only needed if not calling read()
```

The four `Arc<Mutex>` fields are independent — reads don't block writes, meter reads don't block data reads. `WingConsole` is `Clone` (all clones share the same underlying connection).

## Protocols

The Wing supports three protocols:
1. **Native** (binary, TCP/UDP port 2222) — implemented by libwing, used by all official Behringer Wing apps
2. **OSC** (Open Sound Control) — not implemented by libwing
3. **Discovery** (UDP broadcast) — implemented by libwing

The Native and OSC protocols are documented in `Wing-Remote-Protocols.pdf` (by Patrick-Gilles Maillot). The Discovery protocol is documented in `Discovery.md`.
