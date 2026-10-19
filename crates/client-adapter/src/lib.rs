//! 274bot-owned game transport, DNS and bounded stream custody.
//!
//! The fork sees only the host-network byte and fetch hooks. This crate
//! owns process services and native resources without making the fork depend
//! on the host's transport implementation.

pub mod budget;
pub mod cancel;
pub mod control;
pub mod handoff;
pub mod http;
pub mod https_handshake;
pub mod host_network;
pub mod health;
pub mod ledger;
pub mod park;
pub mod reactor;
pub mod records;
pub mod resolver;
pub mod ring;
pub mod shared;
pub mod service;
pub mod stream_handle;
pub mod tcp;
pub mod tcp_endpoint;
pub mod tls_handshake;
pub mod ws_frames;
pub mod ws_endpoint;
