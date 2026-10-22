//! Demo-vault plumbing the CLI doesn't expose: history, cuts, live takes, order.
use std::path::PathBuf;
use writepad_core::{history, manuscript as ms, profile};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let v = PathBuf::from(&a[0]);
    let r = match a[1].as_str() {
        "history-on" => history::enable(&v),
        "snapshot" => history::snapshot(&v, &a[2]).map(|_| ()),
        "save-cut" => ms::save_cut(&v, &a[2], &a[3]),
        "apply-cut" => ms::apply_cut(&v, &a[2], &a[3]),
        "set-live" => ms::set_live(&v, &a[2], &a[3]),
        "set-order" => ms::set_order(&v, &a[2], &a[3..]),
        "weights" => serde_json::from_str(&a[2]).map_err(|e| e.to_string()).and_then(|w| profile::set_weights(&v, &w)),
        c => Err(format!("unknown command {c}")),
    };
    if let Err(e) = r {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
