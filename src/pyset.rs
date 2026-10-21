//! A set of integers that iterates in the order CPython's `set` does.
//!
//! The STEP clean-up (`step_clean.py`) creates entities while walking sets of entity ids, so
//! the ids it gives them, and so the text it writes, follow CPython's hash-table order. This is
//! the same table: open addressing with 9 linear probes then perturbation, dummies left on
//! removal, and the same growth (CPython `Objects/setobject.c`). Integers hash to themselves.

const MINSIZE: usize = 8;
const LINEAR_PROBES: usize = 9;
const PERTURB_SHIFT: u32 = 5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slot {
    Empty,
    Dummy,
    Key(i64),
}

#[derive(Clone, Debug)]
pub struct PySet {
    table: Vec<Slot>,
    fill: usize,
    used: usize,
}

impl Default for PySet {
    fn default() -> Self {
        PySet { table: vec![Slot::Empty; MINSIZE], fill: 0, used: 0 }
    }
}

fn hash(key: i64) -> u64 {
    // CPython: hash(n) == n for small non-negative ints (and -1 becomes -2).
    if key == -1 { (-2_i64) as u64 } else { key as u64 }
}

impl PySet {
    pub fn len(&self) -> usize {
        self.used
    }

    pub fn is_empty(&self) -> bool {
        self.used == 0
    }

    fn mask(&self) -> usize {
        self.table.len() - 1
    }

    /// The slot holding `key`, if any (`set_lookkey`).
    fn find(&self, key: i64) -> Option<usize> {
        let h = hash(key);
        let mask = self.mask();
        let mut i = (h as usize) & mask;
        let mut perturb = h;
        loop {
            let probes = if i + LINEAR_PROBES <= mask { LINEAR_PROBES } else { 0 };
            for j in 0..=probes {
                match self.table[i + j] {
                    Slot::Empty => return None,
                    Slot::Key(k) if k == key => return Some(i + j),
                    _ => {}
                }
            }
            perturb >>= PERTURB_SHIFT;
            i = (i.wrapping_mul(5).wrapping_add(1).wrapping_add(perturb as usize)) & mask;
        }
    }

    pub fn contains(&self, key: i64) -> bool {
        self.find(key).is_some()
    }

    pub fn add(&mut self, key: i64) {
        let h = hash(key);
        let mask = self.mask();
        let mut i = (h as usize) & mask;
        let mut perturb = h;
        let mut freeslot: Option<usize> = None;
        let slot = 'search: loop {
            let probes = if i + LINEAR_PROBES <= mask { LINEAR_PROBES } else { 0 };
            for j in 0..=probes {
                match self.table[i + j] {
                    Slot::Empty => break 'search i + j,
                    Slot::Key(k) if k == key => return,
                    Slot::Dummy => freeslot = Some(i + j), // the last dummy seen, as CPython
                    _ => {}
                }
            }
            perturb >>= PERTURB_SHIFT;
            i = (i.wrapping_mul(5).wrapping_add(1).wrapping_add(perturb as usize)) & mask;
        };
        if let Some(f) = freeslot {
            self.table[f] = Slot::Key(key);
            self.used += 1;
            return;
        }
        self.table[slot] = Slot::Key(key);
        self.fill += 1;
        self.used += 1;
        if self.fill * 5 >= mask * 3 {
            self.resize(if self.used > 50_000 { self.used * 2 } else { self.used * 4 });
        }
    }

    pub fn discard(&mut self, key: i64) {
        if let Some(i) = self.find(key) {
            self.table[i] = Slot::Dummy;
            self.used -= 1;
        }
    }

    fn resize(&mut self, minused: usize) {
        let mut size = MINSIZE;
        while size <= minused {
            size <<= 1;
        }
        let old = std::mem::replace(&mut self.table, vec![Slot::Empty; size]);
        let mask = size - 1;
        self.fill = self.used;
        for s in old {
            let Slot::Key(key) = s else { continue };
            let h = hash(key);
            let mut i = (h as usize) & mask;
            let mut perturb = h;
            'insert: loop {
                if self.table[i] == Slot::Empty {
                    self.table[i] = Slot::Key(key);
                    break 'insert;
                }
                if i + LINEAR_PROBES <= mask {
                    for j in 1..=LINEAR_PROBES {
                        if self.table[i + j] == Slot::Empty {
                            self.table[i + j] = Slot::Key(key);
                            break 'insert;
                        }
                    }
                }
                perturb >>= PERTURB_SHIFT;
                i = (i.wrapping_mul(5).wrapping_add(1).wrapping_add(perturb as usize)) & mask;
            }
        }
    }

    /// The members in CPython's iteration order.
    pub fn items(&self) -> Vec<i64> {
        self.table.iter().filter_map(|s| if let Slot::Key(k) = s { Some(*k) } else { None }).collect()
    }
}
