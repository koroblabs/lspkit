//! A response is written from its result's JSON, serialized once, and reads back as written. Tests
//! check both what the wire carries and what a test or client reads from it.

use expect_test::expect;
use serde_json::{Value, json};

use crate::jsonrpc::{Error, Id, Message, Response};

fn wire(response: &Response) -> String {
    let mut json = Vec::new();
    Message::Response(response.clone()).write_json(&mut json);
    String::from_utf8(json).expect("JSON is UTF-8")
}

#[test]
fn a_result_an_error_and_null_are_written_and_read_back_as_they_were() {
    let answered = Response::from_ok(Id::Number(7), json!({ "label": "find", "kind": 2 }));
    let failed = Response::from_error(Id::from("x"), Error::method_not_found());
    let nothing = Response::from_ok(Id::Number(8), Value::Null);
    expect![[r#"
        {"jsonrpc":"2.0","result":{"kind":2,"label":"find"},"id":7}
        {"jsonrpc":"2.0","error":{"code":-32601,"message":"Method not found"},"id":"x"}
        {"jsonrpc":"2.0","result":null,"id":8}"#]]
    .assert_eq(&[&answered, &failed, &nothing].map(wire).join("\n"));

    for response in [answered, failed, nothing] {
        let Ok(Message::Response(read)) = Message::parse(wire(&response)) else {
            panic!("a response reads back as one");
        };
        assert_eq!(read, response);
    }
    assert_eq!(
        Response::from_ok(Id::Number(1), json!([1, 2])).result(),
        Some(json!([1, 2]))
    );
}

#[test]
fn an_error_code_is_its_number() {
    use crate::jsonrpc::ErrorCode;
    for code in [
        -32700, -32600, -32601, -32602, -32603, -32002, -32001, -32800, -32801, -32802, -32803, -1,
        42,
    ] {
        assert_eq!(ErrorCode::from(code).code(), code);
    }
    assert_eq!(ErrorCode::ServerError(-32801), ErrorCode::ContentModified);
    assert_eq!(
        ErrorCode::ServerError(-32002),
        ErrorCode::ServerNotInitialized
    );
    assert_ne!(ErrorCode::ServerError(-1), ErrorCode::InternalError);
}
