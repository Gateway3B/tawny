//! The database connection and schema sync.

mod access;

mod connection;

pub use access::{connect, connect_embedded};
pub use connection::*;
