use std::{borrow::Cow, fmt, ops::Range};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, value::RawValue};

use super::{Id, Version};

/// A request, or a notification when it has no id.
///
/// Parameters are kept as JSON text, never as a `Value` tree. A handler parses incoming parameters
/// straight into its type, and a notification's are serialized once from the typed value. A
/// `Value` in between builds every object and string twice, which cost four to five times as much
/// for `Response`. Here it would be paid on every message read and every `publishDiagnostics`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Request {
    jsonrpc: Version,
    method: Cow<'static, str>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    params: Option<Params>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<Id>,
}

/// A request's parameters, as JSON text.
///
/// Parameters read from a message are a range of the message's text, which they keep. A full-sync
/// `didChange` carries the whole document, so copying them out allocated every keystroke's text a
/// second time. Parameters the server builds hold their own serialized text. Either way, `get`
/// returns the JSON, and `serde_json::from_str(params.get())` parses it.
#[derive(Clone)]
pub struct Params(Text);

#[derive(Clone)]
enum Text {
    Own(Box<RawValue>),
    Within {
        message: Box<str>,
        range: Range<usize>,
    },
}

impl Params {
    /// Parameters serialized into their own buffer.
    pub(crate) fn own(raw: Box<RawValue>) -> Params {
        Params(Text::Own(raw))
    }

    /// The parameters at `range` of `message`, a message read as JSON.
    pub(crate) fn within(message: Box<str>, range: Range<usize>) -> Params {
        Params(Text::Within { message, range })
    }

    /// The JSON text.
    pub fn get(&self) -> &str {
        match &self.0 {
            Text::Own(raw) => raw.get(),
            Text::Within { message, range } => &message[range.clone()],
        }
    }

    /// The JSON as a `RawValue`. For parameters kept inside their message, this validates the
    /// JSON again, so keep it to tests and tools and off the per-message path.
    pub fn as_raw(&self) -> &RawValue {
        match &self.0 {
            Text::Own(raw) => raw,
            Text::Within { .. } => serde_json::from_str(self.get())
                .expect("parameters are JSON: they were read as JSON with their message"),
        }
    }
}

impl fmt::Debug for Params {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Params").field(&self.get()).finish()
    }
}

/// The JSON text, as `RawValue` displays.
impl fmt::Display for Params {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.get())
    }
}

/// Compares the text, so equal JSON spelled differently (key order, whitespace) is not equal.
impl PartialEq for Params {
    fn eq(&self, other: &Params) -> bool {
        self.get() == other.get()
    }
}

impl Serialize for Params {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_raw().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Params {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Params, D::Error> {
        Box::<RawValue>::deserialize(deserializer).map(Params::own)
    }
}

impl Request {
    /// Starts building a message to `method`. Without an id, it is a notification.
    pub fn build(method: impl Into<Cow<'static, str>>) -> RequestBuilder {
        RequestBuilder {
            method: method.into(),
            params: None,
            id: None,
        }
    }

    pub(crate) fn from_parts(
        method: Cow<'static, str>,
        id: Option<Id>,
        params: Option<Params>,
    ) -> Request {
        Request {
            jsonrpc: Version,
            method,
            params,
            id,
        }
    }

    pub fn method(&self) -> &str {
        &self.method
    }

    pub fn id(&self) -> Option<&Id> {
        self.id.as_ref()
    }

    /// The parameters, as JSON text. `serde_json::from_str(params.get())` reads them into a type,
    /// and `serde_json::to_value(params)` into a `Value`.
    pub fn params(&self) -> Option<&Params> {
        self.params.as_ref()
    }

    pub fn into_parts(self) -> (Cow<'static, str>, Option<Id>, Option<Params>) {
        (self.method, self.id, self.params)
    }
}

/// See [`Request::build`].
#[derive(Debug)]
pub struct RequestBuilder {
    method: Cow<'static, str>,
    params: Option<Value>,
    id: Option<Id>,
}

impl RequestBuilder {
    #[must_use]
    pub fn id(mut self, id: impl Into<Id>) -> RequestBuilder {
        self.id = Some(id.into());
        self
    }

    #[must_use]
    pub fn params(mut self, params: impl Into<Value>) -> RequestBuilder {
        self.params = Some(params.into());
        self
    }

    #[must_use]
    pub fn finish(self) -> Request {
        let params = self.params.map(|params| {
            Params::own(
                serde_json::value::to_raw_value(&params).expect("a Value always serializes"),
            )
        });
        Request::from_parts(self.method, self.id, params)
    }
}
