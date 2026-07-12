//! Fuzz/property tests for generic-profile delta (mirrors gcf-go FuzzGeneric*):
//!  A. decode_generic_delta / decode_generic_full never panic on arbitrary or
//!     mutated input (they fail closed with an Err, or return).
//!  B. arbitrary string cell values survive the full-wire round-trip
//!     (quoting/escaping) with the pack root preserved.

use gcf::{
    decode_generic_delta, decode_generic_full, encode_generic_full, generic_pack_root, GenericSet,
};
use serde_json::{json, Map, Value};

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
    fn frac(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

const ALPHABET: &[char] = &[
    'a', 'b', 'c', 'X', 'Y', 'Z', '0', '1', '2', '9', ' ', '.', ',', '-', '~', '^', '@', '#', '=',
    '|', '\t', '\n', '\r', '"', '\\', '/', 'é', 'ñ', '中', '🦞',
];

fn rand_str(rng: &mut Rng, maxlen: usize) -> String {
    let n = rng.below(maxlen + 1);
    (0..n).map(|_| ALPHABET[rng.below(ALPHABET.len())]).collect()
}

fn row(a: &str, b: &str, id: i64) -> Map<String, Value> {
    json!({"id": id, "a": a, "b": b}).as_object().unwrap().clone()
}

#[test]
fn fuzz_string_cell_roundtrip() {
    let mut rng = Rng(1234);
    for _ in 0..20_000 {
        let a = rand_str(&mut rng, 20);
        let b = rand_str(&mut rng, 20);
        let s = GenericSet {
            name: "t".into(),
            key: "id".into(),
            fields: vec!["id".into(), "a".into(), "b".into()],
            rows: vec![row(&a, &b, 1), row(&b, &a, 2)],
        };
        let (got, _) = decode_generic_full(&encode_generic_full(&s, "")).expect("round-trip decode");
        assert_eq!(
            generic_pack_root(&got),
            generic_pack_root(&s),
            "a={a:?} b={b:?}"
        );
    }
}

#[test]
fn fuzz_decode_never_panics() {
    let mut rng = Rng(99);
    let seeds = [
        "GCF profile=generic delta=true base_root=a new_root=b key=id\n## added [1]{@id,x}\n1|2\n",
        "GCF profile=generic pack_root=r key=id\n## t [2]{@id,x}\n1|2\n3|4\n",
        "## removed [1]{@id}\n99\n",
        "",
    ];
    for _ in 0..20_000 {
        let data = if rng.frac() < 0.5 {
            rand_str(&mut rng, 80)
        } else {
            let mut chars: Vec<char> = seeds[rng.below(seeds.len())].chars().collect();
            let m = rng.below(6);
            for _ in 0..m {
                if !chars.is_empty() {
                    let i = rng.below(chars.len());
                    chars[i] = ALPHABET[rng.below(ALPHABET.len())];
                }
            }
            chars.into_iter().collect()
        };
        let _ = decode_generic_delta(&data); // must not panic
        let _ = decode_generic_full(&data); // must not panic
    }
}
