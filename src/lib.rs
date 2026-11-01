//! Toy payments engine.
//!
//! The crate wires two Tokio actors together:
//!
//! * [`data_stream`] reads a CSV file as an async stream of [`Message`]s and
//!   forwards them through a bounded channel;
//! * [`accountant`] consumes the messages, applies the business rules to a
//!   [`ClientMap`], and stashes out-of-order disputes in a [`StashTx`].
//!
//! [`report`] renders the final [`ClientMap`] back to CSV.

pub mod accountant;
pub mod data_stream;
pub mod message;
pub mod stash;

pub use accountant::Accountant;
pub use data_stream::StreamError;
pub use message::{Message, MessageType};
pub use stash::StashTx;
