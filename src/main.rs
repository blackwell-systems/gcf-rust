use std::io::{self, Read};
use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: gcf <command>");
        eprintln!("commands: encode, decode, encode-generic, decode-generic, version");
        process::exit(1);
    }

    let cmd = args[1].as_str();

    match cmd {
        "version" => {
            println!("gcf {}", env!("CARGO_PKG_VERSION"));
        }
        "encode-generic" => {
            let input = read_stdin();
            let value: serde_json::Value = serde_json::from_str(&input).unwrap_or_else(|e| {
                eprintln!("error: invalid JSON: {}", e);
                process::exit(1);
            });
            print!("{}", gcf::encode_generic(&value));
        }
        "decode-generic" => {
            let input = read_stdin();
            match gcf::decode_generic(&input) {
                Ok(value) => {
                    println!("{}", serde_json::to_string_pretty(&value).unwrap());
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    process::exit(1);
                }
            }
        }
        "encode" => {
            let input = read_stdin();
            let payload: gcf::Payload = serde_json::from_str(&input).unwrap_or_else(|e| {
                eprintln!("error: invalid JSON payload: {}", e);
                process::exit(1);
            });
            print!("{}", gcf::encode(&payload));
        }
        "decode" => {
            let input = read_stdin();
            match gcf::decode(&input) {
                Ok(payload) => {
                    println!("{}", serde_json::to_string_pretty(&payload).unwrap());
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("unknown command: {}", cmd);
            eprintln!("commands: encode, decode, encode-generic, decode-generic, version");
            process::exit(1);
        }
    }
}

fn read_stdin() -> String {
    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf).unwrap_or_else(|e| {
        eprintln!("error: failed to read stdin: {}", e);
        process::exit(1);
    });
    buf
}
