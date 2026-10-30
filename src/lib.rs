//! [![github]](https://github.com/dannyfiresnake/libwing)&ensp;[![crates-io]](https://crates.io/crates/libwing)&ensp;[![docs-rs]](https://docs.rs/libwing)
//!
//! [github]: https://img.shields.io/badge/github-8da0cb?style=for-the-badge&labelColor=555555&logo=github
//! [crates-io]: https://img.shields.io/badge/crates.io-fc8d62?style=for-the-badge&labelColor=555555&logo=rust
//! [docs-rs]: https://img.shields.io/badge/docs.rs-66c2a5?style=for-the-badge&labelColor=555555&logo=docs.rs
//!
//! # Libwing SDK Documentation
//!
//! Libwing is a C++ library for interfacing with Behringer Wing digital mixing
//! consoles. It provides functionality for discovering Wing consoles on the
//! network, connecting to them, reading/writing console parameters, and receiving
//! any changes made on the mixer itself.
//!
//! There is a C wrapper for this library. It generally follows the C++ API. You
//! can find it in `wing_c_api.h`.
//!
//! ## Basic Concepts
//!
//! The Wing console exposes its functionality through a tree of nodes. Each node has:
//! - A unique numeric ID
//! - A hierarchical path name (like a filesystem path)
//! - A type (string, float, integer, enum, etc.)
//! - Optional min/max values and units
//! - Read/write or read-only access
//!
//! ## Getting Started
//!
//! ### Connecting
//! If you have a Wing's IP address, you can connect to it:
//!
//! ```no_run
//! use libwing::WingConsole;
//! let wing = WingConsole::connect(Some("192.168.1.100"))?;
//! # Ok::<(), libwing::Error>(())
//! ```
//!
//! or just run with no IP address to discover the first Wing console on the network:
//!
//! ```no_run
//! use libwing::WingConsole;
//! let wing = WingConsole::connect(None)?;
//! # Ok::<(), libwing::Error>(())
//! ```
//!
//! There is also `WingConsole::scan()` which can be used to scan for Wing mixers.
//! It probes all active IPv4 broadcast interfaces except loopback and
//! point-to-point interfaces, merges duplicate replies, and uses a shared
//! five-second deadline. On Apple platforms, sockets are explicitly bound to
//! their interfaces so VPN default routes do not capture discovery broadcasts.
//! The subsequent TCP connection still requires the VPN to permit LAN access.
//!
//! ### Communication Model
//!
//! - You can request properties from the Wing device using `WingConsole.request_node_data()`,
//!   which will result in a `WingResponse::NodeData` being sent if your request was for a valid
//!   property. Note that you may get other properties as well, as the Wing device will send
//!   unsolicited property changes, so you may need to filter for your specific property change.
//!   After the NodeData is sent (or not), the Wing device will send a `WingResponse::RequestEnd`
//!   message.
//!
//! - You can request node definitions using `WingConsole::request_node_definition()`, which cause
//!   a `WingResponse::NodeDef` message to be read. **wingschema** uses this request to dump the
//!   schema. Again, unsolicited messages may be sent, so you may need to filter for your specific
//!   NodeDef. After the NodeDef is sent (or not), the Wing device will send a `WingResponse::RequestEnd`
//!
//! - You can set properties using the `WingConsole::set_*()` functions. These do not send any
//!   response back.
//!
//! - `WingConsole::read()` will block and return you messages from the Wing mixer as they come in.
//!   If the device is modified either physically or via another user of the API, the Wing device
//!   sends unsolicited `WingResponse::NodeData(id, data)` messages.
//!
//! - `WingConsole::request_meter()` will ask the Wing to start sending meter level data (the
//!   bouncing green/yellow/red level lights on the mixer). It returns an u16 ID corresponding to
//!   this request. This ID will returned when you read the meters data.
//!
//! - `WingConsole::read_meters()` will block and return you messages from the Wing mixer as they
//!   come in. It includes the ID returned from the `request_meter()` call for you to help correlate.
//!
//! All these calls are thread safe.


mod console;
mod discovery;
mod node;
mod ffi;
mod propmap;

pub use console::{WingConsole, DiscoveryInfo, Meter};
pub use node::{WingNodeDef, WingNodeData, NodeType, NodeUnit};
pub use ffi::{WingConsoleHandle, ResponseHandle};

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid data received")]
    InvalidData,
    #[error("Connection error")]
    ConnectionError,
    #[error("Failed to discover Wing console")]
    DiscoveryError,
}

pub enum WingResponse {
    RequestEnd,
    NodeDef(WingNodeDef),
    NodeData(i32, WingNodeData),
}
