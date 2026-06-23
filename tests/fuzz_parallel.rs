use gcf::{decode_generic, encode_generic, encode_generic_with_options, GenericOptions};
use rayon::prelude::*;
use serde_json::Value;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

const PROGRESS_INTERVAL: usize = 50_000_000;

fn rng_next(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

fn gen_key(rng: &mut u64) -> String {
    let chars: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-.>";
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

fn gen_scalar(rng: &mut u64) -> Value {
    match rng_next(rng) % 6 {
        0 => Value::Null,
        1 => Value::Bool(rng_next(rng) % 2 == 0),
        2 => Value::Number(serde_json::Number::from(
            (rng_next(rng) % 200001) as i64 - 100000,
        )),
        3 => {
            let f = ((rng_next(rng) % 200000) as f64 - 100000.0) / 100.0;
            serde_json::Number::from_f64(f)
                .map(Value::Number)
                .unwrap_or(Value::Null)
        }
        _ => Value::String(gen_string(rng)),
    }
}

fn gen_value(rng: &mut u64, depth: usize, max_depth: usize) -> Value {
    if depth >= max_depth {
        return gen_scalar(rng);
    }
    match rng_next(rng) % 6 {
        0 => Value::Null,
        1 => Value::Bool(rng_next(rng) % 2 == 0),
        2 => gen_scalar(rng),
        3 => Value::String(gen_string(rng)),
        4 => {
            let n = (rng_next(rng) % 6) as usize;
            let mut map = serde_json::Map::new();
            for _ in 0..n {
                let key = gen_key(rng);
                *rng = rng.wrapping_add(key.len() as u64);
                map.insert(key, gen_value(rng, depth + 1, max_depth));
            }
            Value::Object(map)
        }
        5 => {
            let n = (rng_next(rng) % 8) as usize;
            Value::Array(
                (0..n)
                    .map(|_| gen_value(rng, depth + 1, max_depth))
                    .collect(),
            )
        }
        _ => gen_scalar(rng),
    }
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

fn run_parallel(
    name: &str,
    iterations: usize,
    seed_offset: usize,
    gen: impl Fn(&mut u64) -> Value + Sync,
) {
    let start = Instant::now();
    let passed = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);

    (0..iterations).into_par_iter().for_each(|i| {
        let seed = (i + seed_offset) as u64 + 1;
        let mut rng = seed;
        let data = gen(&mut rng);

        // Test both flatten-on and flatten-off.
        for no_flatten in [false, true] {
            let encoded = encode_generic_with_options(&data, &GenericOptions { no_flatten });
            let decoded = match decode_generic(&encoded) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!(
                        "\nFAIL {name} seed={} no_flatten={no_flatten}: decode error: {e}",
                        i + seed_offset
                    );
                    failed.fetch_add(1, Ordering::Relaxed);
                    return;
                }
            };

            if !values_equal(&data, &decoded) {
                eprintln!(
                    "\nFAIL {name} seed={} no_flatten={no_flatten}: mismatch",
                    i + seed_offset
                );
                failed.fetch_add(1, Ordering::Relaxed);
                return;
            }
        }

        let p = passed.fetch_add(1, Ordering::Relaxed) + 1;
        if p % PROGRESS_INTERVAL == 0 {
            let elapsed = start.elapsed().as_secs_f64();
            let total = p + failed.load(Ordering::Relaxed);
            let rate = total as f64 / elapsed;
            let remaining = (iterations - total) as f64 / rate;
            eprint!(
                "\r  {name}: {p}/{iterations} ({:.1}%) {rate:.0}/s ETA {:.0}m   ",
                p as f64 / iterations as f64 * 100.0,
                remaining / 60.0,
            );
            std::io::stderr().flush().ok();
        }
    });

    let elapsed = start.elapsed().as_secs_f64();
    let p = passed.load(Ordering::Relaxed);
    let f = failed.load(Ordering::Relaxed);
    eprintln!(
        "\r  {name}: {p} passed, {f} failed in {elapsed:.1}s ({:.0}/s)                    ",
        p as f64 / elapsed,
    );
    assert_eq!(f, 0, "{name}: {f} failures detected");
}

// JSON: push to 10 billion (seed offset 1.25B to avoid overlap)
#[test]
fn json_10b() {
    let iterations: usize = std::env::var("FUZZ_ITERATIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);
    run_parallel("JSON", iterations, 1_250_000_000, |rng| {
        let v = gen_value(rng, 0, 4);
        let s = serde_json::to_string(&v).unwrap();
        serde_json::from_str(&s).unwrap()
    });
}

// YAML: push to 10 billion (seed offset 1B to avoid overlap)
#[test]
fn yaml_10b() {
    let iterations: usize = std::env::var("FUZZ_ITERATIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);
    run_parallel("YAML", iterations, 1_000_000_000, |rng| {
        let v = gen_value(rng, 0, 3);
        let yaml_str = serde_yaml::to_string(&v).unwrap();
        serde_yaml::from_str(&yaml_str).unwrap_or(Value::Null)
    });
}
