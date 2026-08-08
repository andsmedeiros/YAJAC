pub mod error;
pub mod factories;
pub mod uri_generator;

pub use factories::*;

use std::io::Read;

/// An owned, streamable request or response body — the router's byte-tier currency. Any
/// `Read + Send` source plugs in via `Box::new`: a `File` streamed straight out for a download, a
/// `Cursor` over a serialised document. `Send` carries it across the worker thread a request runs
/// on; every stream is owned, so no borrowed lifetime is threaded through the router.
pub type ByteStream = Box<dyn Read + Send>;
