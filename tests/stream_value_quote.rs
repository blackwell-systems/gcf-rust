//! Losslessness tests for streaming row VALUES with adversarial strings.
//!
//! The streaming encoder emits each row cell via `GcfValue::format`. A previous
//! version only quoted empty / `|` / newline strings, so a STRING value that
//! collides with a non-string token was emitted bare and decoded as the wrong
//! type: `"true"` -> Bool, `"123"` -> Number, `"-"`/`"~"`/`"^"` -> markers, and
//! a leading `@`/`#`/`.` misparsed. These tests assert that adversarial string
//! values round-trip AND keep their String type, mixed with real ints, floats,
//! bools, and null.

use gcf::stream_generic::GcfValue;
use gcf::{decode_generic, GenericStreamEncoder};
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

// A cloneable, shared byte buffer so the test can read what the streaming
// encoder wrote (the encoder owns its writer W).
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

// String values that collide with a non-string token if emitted bare.
const ADVERSARIAL: &[&str] = &[
    "true", "false", // -> Bool
    "123", "4.5", "-7", "0", "1e3", // -> Number
    "-", "~", "^", // -> null / missing / attachment markers
    "@x", "#x", ".x", // leading-special misparse
    "", "a|b", "a,b", // delimiter collisions / empty
    "a\"b", "a\\b", // quote / backslash escaping
    "  spaced  ", // leading/trailing space
    "plain", // control: stays bare
];

#[test]
fn stream_adversarial_string_values_roundtrip() {
    let fields = ["s", "n", "f", "b", "z"];
    let buf = SharedBuf::new();
    let enc = GenericStreamEncoder::new(buf.clone());
    enc.begin_array("rows", &fields);

    let mut expected_rows: Vec<Value> = Vec::new();
    for (i, s) in ADVERSARIAL.iter().enumerate() {
        // Mix the adversarial STRING with a real int, float, bool, and null so
        // the row exercises every GcfValue branch alongside the string.
        let real_int = i as i64;
        let real_float = i as f64 + 0.25;
        let real_bool = i % 2 == 0;
        enc.write_row(&[
            GcfValue::Str((*s).to_string()),
            GcfValue::Int(real_int),
            GcfValue::Float(real_float),
            GcfValue::Bool(real_bool),
            GcfValue::Null,
        ]);
        let mut obj = Map::new();
        obj.insert("s".to_string(), Value::String((*s).to_string()));
        obj.insert("n".to_string(), json!(real_int));
        obj.insert("f".to_string(), json!(real_float));
        obj.insert("b".to_string(), json!(real_bool));
        obj.insert("z".to_string(), Value::Null);
        expected_rows.push(Value::Object(obj));
    }
    enc.end_array();
    enc.close().expect("close");

    let wire = buf.into_string();
    let decoded = decode_generic(&wire).unwrap_or_else(|e| panic!("decode failed: {e}\nwire:\n{wire}"));

    let want = {
        let mut m = Map::new();
        m.insert("rows".to_string(), Value::Array(expected_rows));
        Value::Object(m)
    };
    assert_eq!(decoded, want, "round-trip mismatch\nwire:\n{wire}");
}

// A string that spells "true" must decode back to a String, never a Bool.
#[test]
fn stream_string_true_stays_string() {
    let buf = SharedBuf::new();
    let enc = GenericStreamEncoder::new(buf.clone());
    enc.begin_array("rows", &["val"]);
    enc.write_row(&[GcfValue::Str("true".to_string())]);
    enc.end_array();
    enc.close().expect("close");

    let wire = buf.into_string();
    let decoded = decode_generic(&wire).unwrap_or_else(|e| panic!("decode failed: {e}\nwire:\n{wire}"));

    let val = decoded
        .get("rows")
        .and_then(|r| r.get(0))
        .and_then(|o| o.get("val"))
        .expect("rows[0].val present");
    assert_eq!(
        *val,
        Value::String("true".to_string()),
        "string \"true\" must stay a String, got {val:?}\nwire:\n{wire}"
    );
    assert!(!val.is_boolean(), "string \"true\" leaked as a Bool");
}

// Liveness: the encoder actually wrote a non-empty payload with a header and
// row, so an accidental no-op encoder cannot vacuously pass the round-trip.
#[test]
fn stream_value_quote_liveness() {
    let buf = SharedBuf::new();
    let enc = GenericStreamEncoder::new(buf.clone());
    enc.begin_array("rows", &["val"]);
    enc.write_row(&[GcfValue::Str("123".to_string())]);
    enc.end_array();
    enc.close().expect("close");

    let wire = buf.into_string();
    assert!(wire.contains("GCF profile=generic"), "missing profile header:\n{wire}");
    assert!(wire.contains("## rows"), "missing array header:\n{wire}");
    // "123" is a numeric-looking string and must be quoted on the wire.
    assert!(
        wire.contains("\"123\""),
        "numeric-looking string was emitted bare:\n{wire}"
    );
}
