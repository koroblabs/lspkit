use expect_test::{Expect, expect};
use tokio::io::BufReader;

use super::{Unread, read_within};
use crate::jsonrpc::Message;

/// Every outcome of reading `stream` to its end, one per line, with messages over `largest` bytes
/// skipped.
fn check(stream: &str, largest: usize, expected: Expect) {
    let outcomes = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a runtime")
        .block_on(async {
            let mut reader = BufReader::new(stream.as_bytes());
            let mut outcomes = String::new();
            loop {
                let outcome = read_within(&mut reader, largest).await;
                let line = match &outcome {
                    Ok(Message::Request(request)) => format!("request {}", request.method()),
                    Ok(Message::Response(response)) => format!("response {:?}", response.id()),
                    Err(Unread::Malformed(why)) => format!("malformed: {why}"),
                    Err(Unread::Broken(why)) => format!("broken: {why}"),
                    Err(Unread::Closed) => "closed".to_owned(),
                };
                outcomes.push_str(&line);
                outcomes.push('\n');
                if matches!(outcome, Err(Unread::Closed | Unread::Broken(_))) {
                    break;
                }
            }
            outcomes
        });
    expected.assert_eq(&outcomes);
}

fn framed(body: &str) -> String {
    format!("Content-Length: {}\r\n\r\n{body}", body.len())
}

const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;

#[test]
fn messages_are_read_in_turn_and_blank_lines_between_them_are_noise() {
    let stream = framed(INITIALIZED) + "\r\n\r\n" + &framed(INITIALIZED);
    check(
        &stream,
        1024,
        expect![[r#"
            request initialized
            request initialized
            closed
        "#]],
    );
}

#[test]
fn header_names_are_read_in_any_case_and_other_headers_are_passed_over() {
    let stream = format!(
        "content-length: {}\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{INITIALIZED}",
        INITIALIZED.len()
    );
    check(
        &stream,
        1024,
        expect![[r#"
            request initialized
            closed
        "#]],
    );
}

#[test]
fn a_body_that_is_not_json_is_answered_and_the_stream_goes_on() {
    let stream = framed("{not json") + &framed(INITIALIZED);
    check(
        &stream,
        1024,
        expect![[r#"
            malformed: key must be a string at line 1 column 2
            request initialized
            closed
        "#]],
    );
}

#[test]
fn a_message_over_the_largest_is_read_through_and_the_next_one_is_read() {
    let big = format!(
        r#"{{"jsonrpc":"2.0","method":"big","params":"{}"}}"#,
        "x".repeat(200)
    );
    let stream = framed(&big) + &framed(INITIALIZED);
    check(
        &stream,
        100,
        expect![[r#"
            malformed: a message of 244 bytes, over the 100 this server reads, was skipped
            request initialized
            closed
        "#]],
    );
}

#[test]
fn a_length_far_beyond_the_stream_ends_it_instead_of_being_allocated() {
    // Neither allocated nor waited for: the stream ends before the declared length.
    check(
        "Content-Length: 18446744073709551615\r\n\r\n{}",
        1024,
        expect![[r#"
            closed
        "#]],
    );
}

#[test]
fn a_length_that_is_not_a_number_breaks_the_framing() {
    // The body would be read as headers next, and its JSON, having no newline, would swallow the
    // next message's header and silently lose every later message. Reading stops instead.
    let stream = "Content-Length: twelve\r\n\r\n".to_owned() + INITIALIZED + &framed(INITIALIZED);
    check(
        &stream,
        1024,
        expect![[r#"
            broken: a Content-Length of " twelve"
        "#]],
    );
}

#[test]
fn a_header_block_without_a_length_breaks_the_framing() {
    let stream = "Content-Type: application/vscode-jsonrpc\r\n\r\n".to_owned() + INITIALIZED;
    check(
        &stream,
        1024,
        expect![[r#"
            broken: a header block without Content-Length
        "#]],
    );
}

#[test]
fn a_header_line_with_no_end_is_not_read_without_limit() {
    let stream = "X".repeat(10_000);
    check(
        &stream,
        1024,
        expect![[r#"
            broken: a header line longer than 4096 bytes
        "#]],
    );
}

#[test]
fn a_method_of_null_is_neither_a_request_nor_a_response() {
    // Read as a response it would settle the server's request 5 with `null`.
    check(
        &framed(r#"{"jsonrpc":"2.0","method":null,"id":5}"#),
        1024,
        expect![[r#"
            malformed: a method of null
            closed
        "#]],
    );
}
