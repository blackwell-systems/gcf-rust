//! Property/fuzz test for keyed-tabular map encoding (SPEC 7.2a).
//!
//! Generates random JSON objects whose values are objects (maps-of-objects)
//! with adversarial keys and value cells, encodes them, decodes the raw output,
//! and asserts a lossless round-trip. It also checks the selection invariants
//! from SPEC 7.2a.1:
//!  - an eligible multi-member map renders as a keyed table (`[N:]`);
//!  - a single-member map must NOT key (a one-row table saves nothing);
//!  - a map whose value fields all contain '>' falls back to Section 7.2.

use gcf::stream_generic::GcfValue;
use gcf::{decode_generic, encode_generic, GenericStreamEncoder};
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

// Shared writer so the streaming encoder's output can be inspected after close.
#[derive(Clone)]
struct SharedBuf(Rc<RefCell<Vec<u8>>>);
impl SharedBuf {
    fn new() -> Self {
        SharedBuf(Rc::new(RefCell::new(Vec::new())))
    }
    fn as_string(&self) -> String {
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

// Deterministic xorshift64 so any failure is reproducible (matches the repo Rng).
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
    fn frac(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

// Adversarial characters: delimiters, markers, quotes, whitespace, multibyte.
const ALPHABET: &[char] = &[
    'a', 'b', 'c', 'K', '0', '1', '9', ' ', '.', ',', '-', '~', '^', '@', '#', '=', '|', '>', '"',
    '\\', 'é', '中', '🦞',
];

fn rand_str(rng: &mut Rng, maxlen: usize) -> String {
    let n = rng.below(maxlen + 1);
    (0..n)
        .map(|_| ALPHABET[rng.below(ALPHABET.len())])
        .collect()
}

// A random scalar cell value, biased toward strings that would misparse if
// emitted bare (numeric-like, markers, booleans, delimiters).
fn rand_scalar(rng: &mut Rng) -> Value {
    match rng.below(8) {
        0 => Value::Null,
        1 => Value::Bool(rng.below(2) == 0),
        2 => Value::Number((rng.next_u64() as i64 % 100_000).into()),
        3 => Value::String(String::new()),
        4 => Value::String("true".into()),
        5 => Value::String("-".into()),
        6 => Value::String(format!("{}", rng.next_u64() % 1000)),
        _ => Value::String(rand_str(rng, 6)),
    }
}

// A random value-object field name. Occasionally a bare '>' or a '>'-containing
// name (which cannot be a tabular column) to exercise the fallback path.
fn rand_field(rng: &mut Rng) -> String {
    match rng.below(10) {
        0 => "key".into(), // force key-label collision (SPEC 7.2a.2)
        1 => "a>b".into(), // '>' name cannot be a column (SPEC 7.4.6.1.4)
        2 => rand_str(rng, 4),
        _ => {
            // A short bare-ish name from a stable pool for schema overlap.
            const POOL: &[&str] = &["cpu", "mem", "status", "id", "name", "val", "x"];
            POOL[rng.below(POOL.len())].into()
        }
    }
}

// A random member key, biased toward keys that must be quoted to round-trip as a
// string (numeric-like, empty, markers, containing the pipe delimiter).
fn rand_member_key(rng: &mut Rng) -> String {
    match rng.below(10) {
        0 => String::new(),
        1 => "42".into(),
        2 => "-".into(),
        3 => "true".into(),
        4 => "a|b".into(),
        5 => "@x".into(),
        _ => rand_str(rng, 5),
    }
}

// Build a random object value (possibly nested one level: a scalar, or a small
// nested object/array) so flatten/attachment paths are exercised inside members.
fn rand_value_object(rng: &mut Rng, fields: &[String]) -> Value {
    let mut obj = Map::new();
    for f in fields {
        // Each member may omit a field (absent) to exercise the field union.
        if rng.frac() < 0.15 {
            continue;
        }
        let v = match rng.below(12) {
            10 => {
                // Nested object (uniform shape encourages flattening).
                let mut sub = Map::new();
                sub.insert("p".into(), rand_scalar(rng));
                sub.insert("q".into(), rand_scalar(rng));
                Value::Object(sub)
            }
            11 => Value::Array(vec![rand_scalar(rng), rand_scalar(rng)]),
            _ => rand_scalar(rng),
        };
        obj.insert(f.clone(), v);
    }
    Value::Object(obj)
}

fn build_map(rng: &mut Rng) -> Value {
    // 1..=5 members. Choose 1..=4 shared value fields.
    let n_members = 1 + rng.below(5);
    let n_fields = 1 + rng.below(4);
    let mut fields: Vec<String> = Vec::new();
    for _ in 0..n_fields {
        let f = rand_field(rng);
        if !fields.contains(&f) {
            fields.push(f);
        }
    }

    let mut map = Map::new();
    for _ in 0..n_members {
        let k = rand_member_key(rng);
        // Duplicate keys collapse in a JSON object; that's fine, the input is
        // still a valid distinct-key object after insertion.
        // Occasionally shuffle which fields a member carries.
        let member_fields: Vec<String> = fields
            .iter()
            .filter(|_| rng.frac() > 0.1)
            .cloned()
            .collect();
        let use_fields = if member_fields.is_empty() {
            fields.clone()
        } else {
            member_fields
        };
        map.insert(k, rand_value_object(rng, &use_fields));
    }
    Value::Object(map)
}

/// Recompute keyed-map eligibility independently of the encoder, mirroring
/// SPEC 7.2a.1, so the test asserts the wire form matches the selection rule
/// rather than trusting the encoder's own decision.
fn eligible(map: &Map<String, Value>) -> bool {
    if map.len() < 2 {
        return false;
    }
    let mut union: Vec<&String> = Vec::new();
    for v in map.values() {
        match v.as_object() {
            None => return false,
            Some(o) => {
                for f in o.keys() {
                    if !union.contains(&f) {
                        union.push(f);
                    }
                }
            }
        }
    }
    if union.is_empty() {
        return false;
    }
    union.iter().any(|f| !f.contains('>'))
}

#[test]
fn fuzz_keyed_map_roundtrip() {
    let mut rng = Rng(0x5eed_1234);
    let iterations = 200_000;
    let mut keyed_seen = 0usize;
    let mut fallback_seen = 0usize;
    let mut single_seen = 0usize;

    for it in 0..iterations {
        let data = build_map(&mut rng);
        let map = data.as_object().unwrap();

        let encoded = encode_generic(&data).unwrap();

        // Round-trip: decode the RAW encoder output, compare structurally.
        let decoded = decode_generic(&encoded).unwrap_or_else(|e| {
            panic!(
                "iter {}: decode failed: {}\n  input: {}\n  gcf: {:?}",
                it, e, data, encoded
            )
        });
        assert_eq!(
            data, decoded,
            "iter {}: round-trip mismatch\n  gcf: {:?}",
            it, encoded
        );

        // Selection invariant: an eligible TOP-LEVEL map must produce a keyed
        // table as the top-level construct (an anonymous root `## [N:]`).
        let root_keyed = encoded
            .lines()
            .any(|l| l.starts_with("## [") && l.contains(":]{"));
        if eligible(map) {
            assert!(
                root_keyed,
                "iter {}: eligible root map did not produce a root keyed table\n  input: {}\n  gcf: {:?}",
                it, data, encoded
            );
            keyed_seen += 1;
        } else if map.len() < 2 {
            // A single-member wrapper never keys at its own level, but its inner
            // map may key as a named block; only the root form is forbidden here.
            assert!(
                !root_keyed,
                "iter {}: single-member root produced a root keyed table\n  input: {}\n  gcf: {:?}",
                it, data, encoded
            );
            single_seen += 1;
        } else {
            // All-'>' or non-object-valued multi-member map: falls back to §7.2,
            // so no root keyed table.
            assert!(
                !root_keyed,
                "iter {}: fallback root produced a root keyed table\n  input: {}\n  gcf: {:?}",
                it, data, encoded
            );
            fallback_seen += 1;
        }

        // Any `[N:]` header anywhere in the output MUST declare at least two
        // fields (key + >=1 value) and a member count >= 2: a single-member map
        // must never key, at any nesting level (SPEC 7.2a.1, 7.2a.2).
        for l in encoded.lines() {
            let t = l.trim_start();
            if let Some(open) = t.find('[') {
                if t.starts_with("## ") {
                    if let Some(colon) = t[open..].find(":]{") {
                        let count_str = &t[open + 1..open + colon];
                        let n: i64 = count_str.parse().unwrap_or(-1);
                        assert!(
                            n >= 2,
                            "iter {}: keyed header with count {} (< 2)\n  gcf: {:?}",
                            it,
                            count_str,
                            encoded
                        );
                        let brace = &t[open + colon + 2..]; // after ":]"
                        let fields = brace.trim_start_matches('{').trim_end_matches('}');
                        assert!(
                            fields.split(',').count() >= 2,
                            "iter {}: keyed header with < 2 fields\n  gcf: {:?}",
                            it,
                            encoded
                        );
                    }
                }
            }
        }
    }

    // Confirm the corpus actually exercised each branch, so a trivially-passing
    // generator can't hide a broken path.
    assert!(keyed_seen > 1000, "too few keyed cases: {}", keyed_seen);
    assert!(
        single_seen > 1000,
        "too few single-member cases: {}",
        single_seen
    );
    assert!(
        fallback_seen > 100,
        "too few fallback cases: {}",
        fallback_seen
    );
    eprintln!(
        "keyed_map_fuzz: {} iters (keyed={}, single={}, fallback={})",
        iterations, keyed_seen, single_seen, fallback_seen
    );
}

#[test]
fn streaming_keyed_map_roundtrip() {
    // The streaming encoder emits `[?:]` with the key column streamed as cell 0;
    // the decoder reconstructs the object and ignores the summary trailer.
    let buf = SharedBuf::new();
    let enc = GenericStreamEncoder::new(buf.clone());
    enc.begin_keyed_map("servers", "key", &["cpu", "mem", "status"]);
    enc.write_row(&["web-01".into(), 23.into(), 61.into(), "ok".into()]);
    enc.write_row(&["db-01".into(), 41.into(), 83.into(), "ok".into()]);
    enc.write_row(&["cache-1".into(), 67.into(), 52.into(), "warn".into()]);
    enc.end_array();
    enc.close().unwrap();

    let wire = buf.as_string();
    assert!(
        wire.contains("## servers [?:]{key,cpu,mem,status}"),
        "streaming header shape wrong:\n{}",
        wire
    );
    assert!(
        wire.contains("##! summary counts=3"),
        "missing trailer:\n{}",
        wire
    );

    // Decode the encoder's own output.
    let decoded = decode_generic(&wire).unwrap();
    let expected = json!({
        "servers": {
            "web-01": {"cpu": 23, "mem": 61, "status": "ok"},
            "db-01": {"cpu": 41, "mem": 83, "status": "ok"},
            "cache-1": {"cpu": 67, "mem": 52, "status": "warn"},
        }
    });
    assert_eq!(expected, decoded);

    // A '>' value field is rejected and surfaced at close().
    let buf2 = SharedBuf::new();
    let enc2 = GenericStreamEncoder::new(buf2);
    enc2.begin_keyed_map("m", "key", &["a>b"]);
    enc2.write_row(&["k".into(), GcfValue::Null]);
    assert!(enc2.close().is_err(), "'>' value field must be rejected");
}
