//! The database connection and schema sync.

mod access;
mod connection;

pub use access::connect;
pub use connection::*;
