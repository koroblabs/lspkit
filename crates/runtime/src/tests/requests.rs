//! A request's parameters keep their exact text inside the message, and read correctly every way a
//! server reads them.

use expect_test::expect;

use crate::{jsonrpc::Message, protocol::ls_types::DidChangeTextDocumentParams};

fn request(json: &str) -> crate::jsonrpc::Request {
    match Message::parse(json.to_owned()).expect("a message") {
        Message::Request(request) => request,
        Message::Response(_) => panic!("a request was read as a response"),
    }
}

#[test]
fn parameters_are_the_exact_text_they_arrived_as() {
    let request = request(
        r#"{"jsonrpc":"2.0","method":"textDocument/didChange","params":  {"textDocument":{"uri":"file:///a.txt","version":2},"contentChanges":[{"text":"Привіт \"світе\"\n👋"}]}  ,"id":null}"#,
    );
    let params = request.params().expect("parameters");
    expect![[r#"{"textDocument":{"uri":"file:///a.txt","version":2},"contentChanges":[{"text":"Привіт \"світе\"\n👋"}]}"#]]
        .assert_eq(params.get());
    assert_eq!(params.as_raw().get(), params.get());
    assert_eq!(params.to_string(), params.get());

    let typed: DidChangeTextDocumentParams =
        serde_json::from_str(params.get()).expect("read into the handler's type");
    assert_eq!(typed.content_changes[0].text, "Привіт \"світе\"\n👋");
    let value = serde_json::to_value(params).expect("read into a Value");
    assert_eq!(value["textDocument"]["version"], 2);
    assert_eq!(request.id(), None, "an id of null is a notification");
}

#[test]
fn null_or_missing_parameters_are_none() {
    assert!(
        request(r#"{"jsonrpc":"2.0","method":"exit","params":null}"#)
            .params()
            .is_none()
    );
    assert!(
        request(r#"{"jsonrpc":"2.0","method":"exit"}"#)
            .params()
            .is_none()
    );
}

#[test]
fn a_request_written_again_is_the_request_it_was() {
    let json = r#"{"jsonrpc":"2.0","method":"textDocument/hover","params":{"position":{"line":1,"character":2}},"id":7}"#;
    let mut written = Vec::new();
    Message::Request(request(json)).write_json(&mut written);
    assert_eq!(String::from_utf8(written).expect("UTF-8"), json);
}
