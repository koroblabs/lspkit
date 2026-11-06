use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeStruct as _};
use serde_json::{Value, value::RawValue};

use super::{Error, Id, Result};

/// The response to one request.
///
/// The result is kept as JSON text, serialized once from the handler's value. Going through a
/// `serde_json::Value` first built every object and string twice. On a 500-item completion list
/// (143 KB, phrost's size on a real project) that took 519 µs instead of 123 µs, more than the
/// request's own analysis.
#[derive(Clone, Debug)]
pub struct Response {
    kind: Kind,
    id: Id,
}

#[derive(Clone, Debug)]
enum Kind {
    Ok(Box<RawValue>),
    Err(Error),
}

impl PartialEq for Response {
    fn eq(&self, other: &Response) -> bool {
        self.id == other.id
            && match (&self.kind, &other.kind) {
                (Kind::Ok(a), Kind::Ok(b)) => a.get() == b.get(),
                (Kind::Err(a), Kind::Err(b)) => a == b,
                _ => false,
            }
    }
}

impl Response {
    pub fn from_ok(id: Id, result: Value) -> Response {
        Response::from_raw(id, raw(&result))
    }

    /// A response with an already serialized result. The loop answers this way, with no `Value`.
    pub(crate) fn from_raw(id: Id, result: Box<RawValue>) -> Response {
        Response {
            kind: Kind::Ok(result),
            id,
        }
    }

    pub fn from_error(id: Id, error: Error) -> Response {
        Response {
            kind: Kind::Err(error),
            id,
        }
    }

    pub fn from_parts(id: Id, body: Result<Value>) -> Response {
        match body {
            Ok(result) => Response::from_ok(id, result),
            Err(error) => Response::from_error(id, error),
        }
    }

    pub fn into_parts(self) -> (Id, Result<Value>) {
        let body = match self.kind {
            Kind::Ok(result) => Ok(parsed(&result)),
            Kind::Err(error) => Err(error),
        };
        (self.id, body)
    }

    /// The result as JSON text, for the server's own requests, so the caller parses it straight
    /// into its type without a `Value` in between.
    pub(crate) fn into_raw(self) -> Result<Box<RawValue>> {
        match self.kind {
            Kind::Ok(result) => Ok(result),
            Kind::Err(error) => Err(error),
        }
    }

    /// The length of the result's JSON, or 0 for an error. Used to size the frame.
    pub(crate) fn raw_len(&self) -> usize {
        match &self.kind {
            Kind::Ok(result) => result.get().len(),
            Kind::Err(_) => 0,
        }
    }

    pub fn is_ok(&self) -> bool {
        matches!(self.kind, Kind::Ok(_))
    }

    pub fn is_error(&self) -> bool {
        !self.is_ok()
    }

    /// The result, parsed from its JSON text on every call. Meant for tests and one-off reads.
    pub fn result(&self) -> Option<Value> {
        match &self.kind {
            Kind::Ok(result) => Some(parsed(result)),
            Kind::Err(_) => None,
        }
    }

    pub fn error(&self) -> Option<&Error> {
        match &self.kind {
            Kind::Err(error) => Some(error),
            Kind::Ok(_) => None,
        }
    }

    pub fn id(&self) -> &Id {
        &self.id
    }
}

fn raw(value: &Value) -> Box<RawValue> {
    serde_json::value::to_raw_value(value).expect("a Value always serializes")
}

fn parsed(result: &RawValue) -> Value {
    serde_json::from_str(result.get()).expect("a result is JSON: it was written as JSON")
}

impl Serialize for Response {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut out = serializer.serialize_struct("Response", 3)?;
        out.serialize_field("jsonrpc", "2.0")?;
        match &self.kind {
            Kind::Ok(result) => out.serialize_field("result", result)?,
            Kind::Err(error) => out.serialize_field("error", error)?,
        }
        out.serialize_field("id", &self.id)?;
        out.end()
    }
}

/// Reads a response on its own, through a `Value`, so it works with any deserializer. The loop does
/// not use this: it parses every message in one pass and keeps the result as JSON
/// (`Message::parse`).
impl<'de> Deserialize<'de> for Response {
    fn deserialize<D: Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Response, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            #[serde(rename = "jsonrpc")]
            _version: super::Version,
            #[serde(default)]
            result: Option<Value>,
            #[serde(default)]
            error: Option<Error>,
            id: Id,
        }
        let wire = Wire::deserialize(deserializer)?;
        match (wire.result, wire.error) {
            (_, Some(error)) => Ok(Response::from_error(wire.id, error)),
            (Some(result), None) => Ok(Response::from_ok(wire.id, result)),
            // `"result": null` reads as a missing field through `Option`; null is a result.
            (None, None) => Ok(Response::from_ok(wire.id, Value::Null)),
        }
    }
}
