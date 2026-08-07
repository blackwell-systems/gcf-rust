//! Losslessness tests for quoted field names containing MULTIBYTE characters.
//!
//! A quoted field name that needs multibyte bytes (e.g. `"9中"`) produces a
//! header `## [N]{...,"9中"}`. `find_closing_brace` previously returned a CHAR
//! index while its callers sliced by BYTE offset, so any multibyte char in the
//! declaration truncated the brace slice and `decode_generic` returned
//! "invalid field declaration". These tests assert buffered and streaming
//! multibyte field names round-trip. (go/python/ts/swift/kotlin already handle
//! this; the bug was rust-specific.)

use gcf::stream_generic::GcfValue;
use gcf::{decode_generic, encode_generic, GenericStreamEncoder};
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

fn buffered_roundtrip(field: &str) {
    // A uniform array of objects keyed by the multibyte field name forces the
    // tabular header with the quoted field declaration.
    let arr = Value::Array(vec![json!({ field: 1 }), json!({ field: 2 })]);
    let mut top = Map::new();
    top.insert("rows".to_string(), arr);
    let input = Value::Object(top);

    let wire = encode_generic(&input);
    let decoded = decode_generic(&wire)
        .unwrap_or_else(|e| panic!("decode failed for field {field:?}: {e}\nwire:\n{wire}"));
    assert_eq!(
        decoded, input,
        "round-trip mismatch for field {field:?}\nwire:\n{wire}"
    );
}

#[test]
fn buffered_multibyte_field_names_roundtrip() {
    // 3-byte CJK, 4-byte emoji, and a multibyte name containing a comma (which
    // forces quoting on top of the multibyte handling).
    buffered_roundtrip("9中");
    buffered_roundtrip("9😀");
    buffered_roundtrip("中,x");
}

// A cloneable, shared byte buffer so the test can read the streaming output.
#[derive(Clone)]
struct SharedBuf(Rc<RefCell<Vec<u8>>>);
impl SharedBuf {
    fn new() -> Self {
        SharedBuf(Rc::new(RefCell::new(Vec::new())))
    }
    fn into_string(self) -> String {
        String::from_utf8(self.0.borrow().clone()).unwrap()
    }
}
impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// A multibyte field name (not containing '>') must round-trip through the
// streaming encoder too, since it shares `find_closing_brace` on decode.
#[test]
fn streaming_multibyte_field_name_roundtrips() {
    let field = "9中";
    let buf = SharedBuf::new();
    let enc = GenericStreamEncoder::new(buf.clone());
    enc.begin_array("rows", &[field]);
    enc.write_row(&[GcfValue::Int(1)]);
    enc.write_row(&[GcfValue::Int(2)]);
    enc.end_array();
    enc.close().expect("close");

    let wire = buf.into_string();
    let decoded = decode_generic(&wire)
        .unwrap_or_else(|e| panic!("streaming decode failed: {e}\nwire:\n{wire}"));

    let want = json!({ "rows": [ { field: 1 }, { field: 2 } ] });
    assert_eq!(
        decoded, want,
        "streaming round-trip mismatch\nwire:\n{wire}"
    );
}
