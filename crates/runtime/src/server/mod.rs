//! The server side of the protocol: the trait a server implements, the client it sends messages
//! through, and the service the transport drives.

mod client;
mod methods;
mod service;
mod socket;
mod state;

pub use self::{
    client::Client,
    methods::LanguageServer,
    service::{FromParams, LspService, LspServiceBuilder, Method},
    socket::{ClientSocket, RequestStream, ResponseSink},
};
