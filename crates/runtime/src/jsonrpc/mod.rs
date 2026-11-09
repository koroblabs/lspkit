//! JSON-RPC 2.0 as the Language Server Protocol uses it: ids, requests, responses and errors.
//!
//! The names and shapes follow `tower-lsp-server` (MIT OR Apache-2.0). Servers were written against
//! it, so replacing its loop with lspkit's changed none of their handlers.

mod error;
mod request;
mod response;

use std::{
    borrow::Cow,
    fmt::{self, Display},
    marker::PhantomData,
};

use serde::{
    Deserialize, Serialize,
    de::{self, Visitor},
};
use serde_json::Value;

pub use self::{
    error::{Error, ErrorCode},
    request::{Params, Request, RequestBuilder},
    response::Response,
};

/// The result of a request handler.
pub type Result<T> = std::result::Result<T, Error>;

/// A request's id: a number, a string, or `null` for a response to a message that could not be
/// read far enough to find one.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Id {
    Number(i64),
    String(String),
    Null,
}

impl Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Id::Number(id) => Display::fmt(id, f),
            Id::String(id) => fmt::Debug::fmt(id, f),
            Id::Null => f.write_str("null"),
        }
    }
}

impl From<i64> for Id {
    fn from(id: i64) -> Id {
        Id::Number(id)
    }
}

impl From<&str> for Id {
    fn from(id: &str) -> Id {
        Id::String(id.to_owned())
    }
}

impl From<String> for Id {
    fn from(id: String) -> Id {
        Id::String(id)
    }
}

impl From<ls_types::NumberOrString> for Id {
    fn from(id: ls_types::NumberOrString) -> Id {
        match id {
            ls_types::NumberOrString::Number(id) => Id::Number(id.into()),
            ls_types::NumberOrString::String(id) => Id::String(id),
        }
    }
}

/// One message on the wire, in either direction.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Message {
    Request(Request),
    Response(Response),
}

impl Message {
    /// Parses one message from its JSON in one pass. A request has a method; a response does not.
    ///
    /// It reads into a struct with every field of either kind. Reading into a `Value` first to find
    /// `method` built the whole parameter tree twice, and the handler then parsed it a third time.
    /// Parameters and results stay JSON text here and are parsed once, by whoever needs them.
    ///
    /// A request's parameters stay in `body`, which the request then owns.
    pub(crate) fn parse(body: String) -> std::result::Result<Message, serde_json::Error> {
        let (method, id, params, result, error) = {
            let wire: Wire<'_> = serde_json::from_str(&body)?;
            let params = wire.params.map(|params| {
                let start = params.get().as_ptr().addr() - body.as_ptr().addr();
                start..start + params.get().len()
            });
            let method = match wire.method {
                MethodName::Absent => None,
                MethodName::Null => return Err(de::Error::custom("a method of null")),
                MethodName::Named(name) => Some(intern(name)),
            };
            (method, wire.id, params, wire.result, wire.error)
        };
        Ok(match method {
            // A request with `"id": null` is read as a notification, as the old loop did.
            Some(method) => Message::Request(Request::from_parts(
                method,
                id.filter(|id| *id != Id::Null),
                params.map(|range| Params::within(body.into_boxed_str(), range)),
            )),
            None => {
                let Some(id) = id else {
                    return Err(serde::de::Error::missing_field("id"));
                };
                Message::Response(match (result, error) {
                    (_, Some(error)) => Response::from_error(id, error),
                    (Some(result), None) => Response::from_raw(id, result),
                    // `"result": null` reads as a missing field through `Option`; null is a result.
                    (None, None) => Response::from_ok(id, Value::Null),
                })
            }
        })
    }

    /// Writes the message's JSON at the end of `out`.
    pub(crate) fn write_json(&self, out: &mut Vec<u8>) {
        let written = match self {
            Message::Request(request) => serde_json::to_writer(&mut *out, request),
            Message::Response(response) => serde_json::to_writer(&mut *out, response),
        };
        written.expect("a message serializes: every field is JSON already");
    }

    /// Roughly the length of the message's JSON: its payload, which is nearly all of a large
    /// message, plus 64 bytes for the envelope (`{"jsonrpc":"2.0", ...}` and the id).
    pub(crate) fn size_hint(&self) -> usize {
        let carried = match self {
            Message::Request(request) => {
                request.method().len() + request.params().map_or(0, |params| params.get().len())
            }
            Message::Response(response) => response.raw_len(),
        };
        carried + 64
    }
}

/// A request's method as it is kept: the protocol's own name, `'static`, for the methods the
/// handler table knows (`server::known`), and a copy for any other. Every message used to
/// allocate a `String` for a name the table then compared against literals.
fn intern(name: Cow<'_, str>) -> Cow<'static, str> {
    match crate::server::known(&name) {
        Some(known) => Cow::Borrowed(known),
        None => Cow::Owned(name.into_owned()),
    }
}

/// Every field a message of either kind may have, for [`Message::parse`].
#[derive(Deserialize)]
struct Wire<'a> {
    #[serde(rename = "jsonrpc")]
    _version: Version,
    /// Absent for a response, `Null` for `"method": null`, which is neither a request nor a
    /// response, and otherwise the name, borrowed from the message text.
    #[serde(default, borrow)]
    method: MethodName<'a>,
    /// Borrowed from the message text, which the request then owns.
    #[serde(default, borrow)]
    params: Option<&'a serde_json::value::RawValue>,
    /// `Some(Id::Null)` for `"id": null`, which a response to an unreadable message carries, and
    /// `None` only when there is no id at all.
    #[serde(default, deserialize_with = "present")]
    id: Option<Id>,
    #[serde(default)]
    result: Option<Box<serde_json::value::RawValue>>,
    #[serde(default)]
    error: Option<Error>,
}

/// The `method` field as read: not there, `null`, or a name. The name is borrowed from the message
/// when it has no escapes, which is every method name the protocol defines, and copied otherwise.
#[derive(Default)]
enum MethodName<'a> {
    #[default]
    Absent,
    Null,
    Named(Cow<'a, str>),
}

impl<'de: 'a, 'a> Deserialize<'de> for MethodName<'a> {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<MethodName<'a>, D::Error> {
        struct Name<'a>(PhantomData<&'a ()>);

        impl<'de: 'a, 'a> Visitor<'de> for Name<'a> {
            type Value = MethodName<'a>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a method name")
            }

            fn visit_borrowed_str<E: de::Error>(
                self,
                name: &'de str,
            ) -> std::result::Result<Self::Value, E> {
                Ok(MethodName::Named(Cow::Borrowed(name)))
            }

            fn visit_str<E: de::Error>(self, name: &str) -> std::result::Result<Self::Value, E> {
                Ok(MethodName::Named(Cow::Owned(name.to_owned())))
            }

            fn visit_string<E: de::Error>(
                self,
                name: String,
            ) -> std::result::Result<Self::Value, E> {
                Ok(MethodName::Named(Cow::Owned(name)))
            }

            fn visit_unit<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(MethodName::Null)
            }
        }

        deserializer.deserialize_any(Name(PhantomData))
    }
}

/// Reads a present field as `Some`, even when it holds `null`, which `Option` alone reads as
/// absent.
fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> std::result::Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

/// The `"jsonrpc": "2.0"` every message carries, checked on the way in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Version;

impl Serialize for Version {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str("2.0")
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Version, D::Error> {
        let version = String::deserialize(deserializer)?;
        if version == "2.0" {
            Ok(Version)
        } else {
            Err(serde::de::Error::custom(format!(
                "expected JSON-RPC 2.0, got {version:?}"
            )))
        }
    }
}
