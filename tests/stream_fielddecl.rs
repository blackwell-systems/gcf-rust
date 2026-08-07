//! Property/fuzz tests for the streaming tabular header
//! (`GenericStreamEncoder::begin_array`) with adversarial field names: comma,
//! pipe, quote, empty, leading @/#/., spaces. The header previously joined
//! field names raw, so such a name produced an invalid or ambiguous field
//! declaration (SPEC 8.3). Field names now format via `format_key` (Section
//! 2.4), matching the buffered tabular header. A field name containing '>' is
//! rejected (a flattened path is not representable in a flat streaming row);
//! that path is asserted separately.

use gcf::stream_generic::GcfValue;
use gcf::{decode_generic, GenericStreamEncoder};
use serde_json::{Map, Value};
use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

const DEFAULT_ITERATIONS: usize = 200_000;

fn get_iterations() -> usize {
    std::env::var("GCF_ITERATIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_ITERATIONS)
}

// Deterministic xorshift64 so any failure is reproducible.
struct Rng(u64);
impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

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

// Field-name alphabet including every character that stresses header quoting:
// the two GCF delimiters (comma separates fields, pipe separates row cells),
// the quote char, whitespace, and characters that make a key non-bare when
// leading. '>' is deliberately excluded (it is rejected, tested separately).
const NAME_CHARS: &[char] = &[
    'a', 'b', 'X', '0', '9', ',', '|', '"', ' ', '.', '@', '#', '-', '_',
];

fn gen_key(rng: &mut Rng) -> String {
    // Bias toward the empty name (needs quoting) and short names so the
    // delimiter/quote characters appear frequently.
    let n = rng.below(6); // 0..=5
    (0..n)
        .map(|_| NAME_CHARS[rng.below(NAME_CHARS.len())])
        .collect()
}

fn needs_quoting(f: &str) -> bool {
    f.is_empty() || f.contains(',') || f.contains('|') || f.contains('"')
}

fn assemble(body: &str) -> String {
    // The streaming encoder emits the GCF profile line itself; use its output as-is.
    body.to_string()
}

#[test]
fn fuzz_stream_field_names_roundtrip() {
    let iterations = get_iterations();
    let mut rng = Rng(0x5738);
    let mut saw_special = false;

    for i in 0..iterations {
        // 1..=5 distinct field names.
        let nf = 1 + rng.below(5);
        let mut fields: Vec<String> = Vec::new();
        while fields.len() < nf {
            let f = gen_key(&mut rng);
            if f.contains('>') {
                continue; // '>' is rejected, tested separately
            }
            if fields.iter().any(|x| x == &f) {
                continue;
            }
            if needs_quoting(&f) {
                saw_special = true;
            }
            fields.push(f);
        }
        let field_refs: Vec<&str> = fields.iter().map(|s| s.as_str()).collect();

        // 1..=6 rows of integer cells (unambiguous round-trip; the field name
        // is the axis under test).
        let nr = 1 + rng.below(6);
        let buf = SharedBuf::new();
        let enc = GenericStreamEncoder::new(buf.clone());
        enc.begin_array("rows", &field_refs);
        let mut expected_rows: Vec<Value> = Vec::with_capacity(nr);
        for r in 0..nr {
            let mut cells: Vec<GcfValue> = Vec::with_capacity(fields.len());
            let mut obj = Map::new();
            for (j, f) in fields.iter().enumerate() {
                let v = (i as i64 * 100 + r as i64 * 10 + j as i64) % 1000;
                cells.push(GcfValue::Int(v));
                obj.insert(f.clone(), Value::from(v));
            }
            enc.write_row(&cells);
            expected_rows.push(Value::Object(obj));
        }
        enc.end_array();
        enc.close()
            .unwrap_or_else(|e| panic!("iter {i}: close: {e}\n fields: {fields:?}"));

        let wire = assemble(&buf.into_string());
        let decoded = decode_generic(&wire).unwrap_or_else(|e| {
            panic!("iter {i}: decode failed: {e}\n fields: {fields:?}\n wire: {wire:?}")
        });

        let want = {
            let mut m = Map::new();
            m.insert("rows".to_string(), Value::Array(expected_rows));
            Value::Object(m)
        };
        assert_eq!(
            decoded, want,
            "iter {i}: round-trip mismatch\n fields: {fields:?}\n wire: {wire:?}"
        );
    }

    assert!(
        saw_special,
        "generator never produced a field name needing quoting (empty / , | \")"
    );
    eprintln!("PASS: {iterations} streaming arrays with adversarial field names round-tripped");
}

// Locks the SPEC 8.3 requirement that a streaming value field name containing
// '>' is rejected. begin_array() has no return value, so the error is surfaced
// at close().
#[test]
fn stream_field_name_gt_rejected() {
    let buf = SharedBuf::new();
    let enc = GenericStreamEncoder::new(buf.clone());
    enc.begin_array("rows", &["id", "a>b"]);
    enc.write_row(&[GcfValue::Int(1), GcfValue::Int(2)]);
    enc.end_array();
    let err = enc.close();
    assert!(
        err.is_err(),
        "expected an error for a '>' field name, got Ok\n wire: {:?}",
        buf.into_string()
    );
}
