//! The base protocol's framing: a `Content-Length` header, a blank line, and that many bytes of
//! JSON.

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt};

use crate::jsonrpc::Message;

#[cfg(test)]
mod tests;

/// The longest header line read. Real headers are short (`Content-Length: 1234`, `Content-Type`),
/// so a line this long means the framing is lost and a body is being read as a header.
const HEADER_LINE: u64 = 4 * 1024;

/// The largest message read into memory.
///
/// The buffer is sized from the client's `Content-Length` before any byte arrives. A length of
/// 100 GB would fail the allocation, which aborts the process with no unwind to catch. A larger
/// message is read and discarded so the stream stays in sync: an editor with a 300 MB log open
/// loses that one message, and the session goes on.
const LARGEST: usize = 256 * 1024 * 1024;

/// Why a message could not be read.
#[derive(Debug)]
pub(crate) enum Unread {
    /// The stream ended or failed; nothing more will come.
    Closed,
    /// The framing is lost, so the start of the next message is unknown and nothing more is read.
    /// Resynchronizing would mean guessing, and a guess that lands inside a body would lose every
    /// later message.
    Broken(String),
    /// One message was malformed; the stream goes on after it.
    Malformed(String),
}

/// Reads the next message, or says why there is none.
pub(crate) async fn read<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Message, Unread> {
    read_within(reader, LARGEST).await
}

/// [`read`] with the size limit as a parameter, so a test can reach it.
async fn read_within<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    largest: usize,
) -> Result<Message, Unread> {
    let mut length: Option<usize> = None;
    let mut headers = false;
    let mut line = String::new();
    loop {
        line.clear();
        let read = (&mut *reader)
            .take(HEADER_LINE)
            .read_line(&mut line)
            .await
            .map_err(|_| Unread::Closed)?;
        if read == 0 {
            return Err(Unread::Closed);
        }
        if !line.ends_with('\n') && read as u64 == HEADER_LINE {
            return Err(Unread::Broken(format!(
                "a header line longer than {HEADER_LINE} bytes"
            )));
        }
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            if length.is_some() {
                break;
            }
            if headers {
                return Err(Unread::Broken(
                    "a header block without Content-Length".into(),
                ));
            }
            // Some clients send blank lines between messages.
            continue;
        }
        headers = true;
        if let Some((name, value)) = header.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().ok();
            if length.is_none() {
                return Err(Unread::Broken(format!("a Content-Length of {value:?}")));
            }
        }
    }
    let length = length.unwrap_or(0);
    if length > largest {
        let skipped = tokio::io::copy(
            &mut (&mut *reader).take(length as u64),
            &mut tokio::io::sink(),
        )
        .await
        .map_err(|_| Unread::Closed)?;
        if skipped < length as u64 {
            return Err(Unread::Closed);
        }
        return Err(Unread::Malformed(format!(
            "a message of {length} bytes, over the {largest} this server reads, was skipped"
        )));
    }
    let mut body = vec![0; length];
    reader
        .read_exact(&mut body)
        .await
        .map_err(|_| Unread::Closed)?;
    let body = String::from_utf8(body)
        .map_err(|_| Unread::Malformed("a message that is not UTF-8".to_owned()))?;
    Message::parse(body).map_err(|err| Unread::Malformed(err.to_string()))
}

/// Space reserved before a message's JSON for its header, which is written once the length is
/// known. It fits the longest possible `Content-Length`.
const HEADER_ROOM: usize = "Content-Length: 18446744073709551615\r\n\r\n".len();

/// One framed message: header and JSON in one buffer, starting at `start`.
///
/// The message is serialized straight into the buffer sent to the client, presized from its
/// payload, so its parameters or result are copied once. Serializing into a separate growing
/// buffer, then copying behind a header and into the queue, took three buffers and two extra
/// copies of a 143 KB completion.
#[derive(Debug)]
pub(crate) struct Frame {
    bytes: Vec<u8>,
    start: usize,
}

impl Frame {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes[self.start..]
    }

    /// The frame's memory, including spare capacity. The queue's bound counts this.
    pub(crate) fn cost(&self) -> usize {
        self.bytes.capacity()
    }

    /// A frame copied from bytes that a writer was given as a slice.
    pub(crate) fn copied(bytes: &[u8]) -> Frame {
        Frame {
            bytes: bytes.to_vec(),
            start: 0,
        }
    }
}

pub(crate) fn frame(message: &Message) -> Frame {
    let mut bytes = Vec::with_capacity(HEADER_ROOM + message.size_hint());
    bytes.resize(HEADER_ROOM, 0);
    message.write_json(&mut bytes);
    let header = format!("Content-Length: {}\r\n\r\n", bytes.len() - HEADER_ROOM);
    let start = HEADER_ROOM - header.len();
    bytes[start..HEADER_ROOM].copy_from_slice(header.as_bytes());
    Frame { bytes, start }
}
