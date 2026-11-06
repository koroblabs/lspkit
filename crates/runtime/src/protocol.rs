//! The API a server is written against: the protocol types, JSON-RPC and the server trait.
//!
//! Servers import these from here and never from a framework; `lspkit-arch` rejects a server crate
//! that names one. The names and signatures are those of `tower-lsp-server`, so replacing it with
//! lspkit's own loop ([`LspService`]) required no change to any handler.

pub use ls_types;

pub use crate::{
    jsonrpc,
    server::{
        Client, ClientSocket, FromParams, LanguageServer, LspService, LspServiceBuilder, Method,
        RequestStream, ResponseSink,
    },
};
