use gcf::{decode_generic, encode_generic};
use rayon::prelude::*;
use serde_json::Value;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

// 1M CSV to close the gap from deleted Python test
const ITERATIONS: usize = 1_000_000;
const PROGRESS_INTERVAL: usize = 500_000;
// Seed offset 334M to avoid overlap with main run
const SEED_OFFSET: usize = 334_000_000;

fn rng_next(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

fn gen_key(rng: &mut u64) -> String {
    let chars: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-.";
    let len = (rng_next(rng) % 14) as usize + 1;
    let mut s = String::with_capacity(len);
    s.push(chars[(rng_next(rng) % 52) as usize] as char);
    for _ in 1..len {
        s.push(chars[(rng_next(rng) % chars.len() as u64) as usize] as char);
    }
    s
}

fn gen_string(rng: &mut u64) -> String {
    let chars: &[u8] =
        b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 _-.,;:/?&=~";
    let len = (rng_next(rng) % 20) as usize;
    let mut s = String::with_capacity(len);
    for _ in 0..len {
        s.push(chars[(rng_next(rng) % chars.len() as u64) as usize] as char);
    }
    s
}

fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => {
            a.as_f64().unwrap_or(f64::NAN) == b.as_f64().unwrap_or(f64::NAN)
        }
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| values_equal(x, y))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.get(k).map_or(false, |bv| values_equal(v, bv)))
        }
        _ => false,
    }
}

#[test]
fn rerun_csv_1m() {
    let start = Instant::now();
    let passed = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);

    (0..ITERATIONS).into_par_iter().for_each(|i| {
        let seed = (i + SEED_OFFSET) as u64 + 1;
        let mut rng = seed;

        let num_rows = (rng_next(&mut rng) % 10) as usize + 1;
        let num_cols = (rng_next(&mut rng) % 5) as usize + 1;
        let mut keys = Vec::new();
        for _ in 0..num_cols {
            let k = gen_key(&mut rng);
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
        if keys.is_empty() {
            passed.fetch_add(1, Ordering::Relaxed);
            return;
        }

        let mut buf = Vec::new();
        {
            let mut wtr = csv::Writer::from_writer(&mut buf);
            wtr.write_record(&keys).unwrap();
            for _ in 0..num_rows {
                let row: Vec<String> = keys.iter().map(|_| gen_string(&mut rng)).collect();
                wtr.write_record(&row).unwrap();
            }
            wtr.flush().unwrap();
        }

        let mut rdr = csv::Reader::from_reader(&buf[..]);
        let headers: Vec<String> = rdr
            .headers()
            .unwrap()
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut rows = Vec::new();
        for result in rdr.records() {
            let record = result.unwrap();
            let mut map = serde_json::Map::new();
            for (j, h) in headers.iter().enumerate() {
                if let Some(val) = record.get(j) {
                    map.insert(h.clone(), Value::String(val.to_string()));
                }
            }
            rows.push(Value::Object(map));
        }
        let data = Value::Array(rows);

        let encoded = encode_generic(&data);
        let decoded = match decode_generic(&encoded) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("\nFAIL CSV seed={}: decode error: {e}", i + SEED_OFFSET);
                failed.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };

        if !values_equal(&data, &decoded) {
            eprintln!("\nFAIL CSV seed={}: mismatch", i + SEED_OFFSET);
            failed.fetch_add(1, Ordering::Relaxed);
            return;
        }

        let p = passed.fetch_add(1, Ordering::Relaxed) + 1;
        if p % PROGRESS_INTERVAL == 0 {
            let elapsed = start.elapsed().as_secs_f64();
            let rate = p as f64 / elapsed;
            eprint!(
                "\r  CSV-rerun: {p}/{ITERATIONS} ({:.1}%) {rate:.0}/s   ",
                p as f64 / ITERATIONS as f64 * 100.0
            );
            std::io::stderr().flush().ok();
        }
    });

    let elapsed = start.elapsed().as_secs_f64();
    let p = passed.load(Ordering::Relaxed);
    let f = failed.load(Ordering::Relaxed);
    eprintln!(
        "\r  CSV-rerun: {p} passed, {f} failed in {elapsed:.1}s ({:.0}/s)                    ",
        p as f64 / elapsed
    );
    assert_eq!(f, 0, "CSV: {f} failures detected");
}
