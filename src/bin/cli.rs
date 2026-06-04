//! Headless interface / test harness for the gold editor core.
//!
//!   bellwright-gold-cli info <save>          print name, village, and current gold
//!   bellwright-gold-cli set  <save> <amount> set gold (backs up to <save>.bak once)

use bellwright_gold_editor::{set_gold_on_disk, SaveFile};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let usage = || {
        eprintln!(
            "usage:\n  bellwright-gold-cli info <save>\n  bellwright-gold-cli set <save> <amount>"
        );
        std::process::exit(2);
    };
    if args.len() < 3 {
        usage();
    }
    match args[1].as_str() {
        "info" => match SaveFile::load(&args[2]).and_then(|s| s.find_gold().map(|g| (s, g))) {
            Ok((s, g)) => {
                println!("name:      {}", s.display_name);
                println!("village:   {}", s.village);
                println!("character: {}", s.character);
                println!("gold:      {}", g.value);
            }
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        },
        "set" => {
            if args.len() < 4 {
                usage();
            }
            let amount: u64 = match args[3].parse() {
                Ok(v) => v,
                Err(_) => {
                    eprintln!("error: amount must be a non-negative integer");
                    std::process::exit(2);
                }
            };
            match set_gold_on_disk(&args[2], amount) {
                Ok(()) => println!("ok: gold set to {amount} (backup at <file>.bak)"),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        _ => usage(),
    }
}
