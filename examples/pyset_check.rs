//! Replays set operations read from stdin ("a 12" adds, "d 12" discards, "p" prints the order)
//! so the order can be compared with CPython's (examples/pyset_check.py).

use std::io::BufRead;

use craftify::pyset::PySet;

fn main() {
    let mut sets: Vec<PySet> = Vec::new();
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.as_slice() {
            ["n"] => sets.push(PySet::default()),
            ["a", k] => sets.last_mut().unwrap().add(k.parse().unwrap()),
            ["d", k] => sets.last_mut().unwrap().discard(k.parse().unwrap()),
            ["p"] => println!("{:?}", sets.last().unwrap().items()),
            _ => {}
        }
    }
}
