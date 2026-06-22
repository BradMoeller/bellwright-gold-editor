//! Headless interface for the Bellwright save editor.
//!
//!   bellwright-gold-cli info       <save>                    print name, village, gold
//!   bellwright-gold-cli set        <save> <amount>           set gold
//!   bellwright-gold-cli set-renown <save> <current> <new>    set renown
//!
//! Renown can only be located by its current value (it is one of thousands of
//! identically-shaped records), so `set-renown` takes the value shown in-game
//! plus the value you want. See ../../bellwright_renown/FINDINGS.md.

use bellwright_gold_editor::{set_gold_on_disk, set_renown_on_disk, SaveFile};

fn parse_amount(s: &str) -> u64 {
    s.parse().unwrap_or_else(|_| {
        eprintln!("error: amount must be a non-negative integer");
        std::process::exit(2);
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let usage = || {
        eprintln!(
            "usage:\n  bellwright-gold-cli info <save>\n  bellwright-gold-cli set <save> <amount>\n  bellwright-gold-cli set-renown <save> <current> <new>"
        );
        std::process::exit(2);
    };
    if args.len() < 3 { usage(); }

    match args[1].as_str() {
        "info" => {
            let s = SaveFile::load(&args[2]).unwrap_or_else(|e| { eprintln!("error: {e}"); std::process::exit(1); });
            let gold = s.find_gold().unwrap_or_else(|e| { eprintln!("error: {e}"); std::process::exit(1); });
            println!("name:      {}", s.display_name);
            println!("village:   {}", s.village);
            println!("character: {}", s.character);
            println!("gold:      {}", gold.value);
            println!("renown:    (enter the in-game value to edit it; see set-renown)");
        }
        "set" => {
            if args.len() < 4 { usage(); }
            let amount = parse_amount(&args[3]);
            match set_gold_on_disk(&args[2], amount) {
                Ok(()) => println!("ok: gold set to {amount} (backup at <file>.bak)"),
                Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
            }
        }
        "set-renown" => {
            if args.len() < 5 { usage(); }
            let current = parse_amount(&args[3]);
            let new = parse_amount(&args[4]);
            match set_renown_on_disk(&args[2], current, new) {
                Ok(()) => println!("ok: renown {current} -> {new} (backup at <file>.bak)"),
                Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
            }
        }
        _ => usage(),
    }
}
