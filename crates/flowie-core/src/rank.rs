//! Fractional indexing: string keys that order siblings (and, later, other
//! user-ordered lists such as stars) so that a new key can always be made
//! between any two neighbours without rewriting anyone else's key.
//!
//! # Key format
//!
//! A key is ASCII over the base-62 alphabet `0-9A-Za-z`, in that (ASCII)
//! order, so comparing keys as bytes, as Rust `str`s and with SQLite's
//! default `BINARY` collation all give the same order. A key is an *integer
//! part* followed by a *fraction*:
//!
//! - The integer part's first character gives its length: `a`..`z` start
//!   integers of 2..27 characters (`a0`, `a1`, …, `az`, `b00`, …), `Z`..`A`
//!   start the negative ones of 2..27 characters (`Zz`, …, `Z0`, `Yzz`, …).
//!   Incrementing and decrementing it is how keys are made at either end of a
//!   list, so appending or prepending thousands of times keeps keys short
//!   (see the tests for measured lengths).
//! - The fraction is any base-62 digit string that does not end in `0`, read
//!   as a number after a "decimal" point. Inserting between two neighbours
//!   takes the midpoint of their fractions.
//!
//! The smallest integer (`A` followed by 26 `0`s) alone is not a valid key,
//! so there is always room before any key.
//!
//! # Invariants
//!
//! - `between(a, b)` returns a valid key `k` with `a < k < b` (byte order);
//!   `None` stands for the start or the end of the list.
//! - Every generated key ends with a random suffix (`SUFFIX_LEN` digits, the
//!   last one non-zero), so two devices inserting at the same place while
//!   offline get different keys (and an equal key is still ordered by node
//!   id). The deterministic part is never a prefix of `b`, which is what
//!   lets the suffix go after it without passing `b`.
//! - No function here ever changes an existing key.
//!
//! The algorithm follows rocicorp's `fractional-indexing` (after Figma's
//! write-up), with two changes: before a key with a fraction it decrements
//! the integer instead of reusing it, and it adds the random suffix.
//!
//! The key string itself is the encoding everywhere (database, sync): other
//! platforms compare the bytes and never regenerate keys. One that needs to
//! make keys must implement this same algorithm.

const DIGITS: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const BASE: usize = DIGITS.len();
const ZERO: u8 = b'0';
const LAST: u8 = b'z';
/// Random digits at the end of every generated key: 62^4 ≈ 14.8 million
/// ways for two concurrent inserts at one place to come out different.
const SUFFIX_LEN: usize = 4;
/// The integer that cannot stand alone as a key.
const SMALLEST_INTEGER: &[u8] = b"A00000000000000000000000000";

fn digit(c: u8) -> Option<usize> {
    match c {
        b'0'..=b'9' => Some((c - b'0') as usize),
        b'A'..=b'Z' => Some((c - b'A') as usize + 10),
        b'a'..=b'z' => Some((c - b'a') as usize + 36),
        _ => None,
    }
}

fn index(c: u8) -> usize {
    digit(c).expect("validated key")
}

/// Length of the integer part that starts with `head`.
fn integer_len(head: u8) -> Option<usize> {
    match head {
        b'a'..=b'z' => Some((head - b'a') as usize + 2),
        b'A'..=b'Z' => Some((b'Z' - head) as usize + 2),
        _ => None,
    }
}

/// Whether `key` is a well-formed key (any key `between` accepts).
pub fn is_valid(key: &str) -> bool {
    let key = key.as_bytes();
    let Some(len) = key.first().and_then(|&h| integer_len(h)) else { return false };
    key.len() >= len
        && key.iter().all(|&c| digit(c).is_some())
        && key != SMALLEST_INTEGER
        // A trailing `0` is fine in the integer part (`a0`), not in the fraction.
        && (key.len() == len || key.last() != Some(&ZERO))
}

/// Split a valid key into (integer part, fraction).
fn split(key: &[u8]) -> (&[u8], &[u8]) {
    key.split_at(integer_len(key[0]).expect("validated key"))
}

/// A fraction strictly between `a` and `b` (`None` = 1), neither of which
/// ends in `0`; the result does not either.
fn midpoint(a: &[u8], b: Option<&[u8]>) -> Vec<u8> {
    if let Some(b) = b {
        // Drop the common prefix, reading missing digits of `a` as 0.
        let n = b.iter().enumerate().take_while(|&(i, &d)| a.get(i).copied().unwrap_or(ZERO) == d).count();
        if n > 0 {
            let mut out = b[..n].to_vec();
            out.extend(midpoint(a.get(n..).unwrap_or(&[]), Some(&b[n..])));
            return out;
        }
    }
    let da = a.first().map_or(0, |&c| index(c));
    let db = b.map_or(BASE, |b| index(b[0]));
    if db - da > 1 {
        vec![DIGITS[(da + db).div_ceil(2)]]
    } else if let Some(b) = b.filter(|b| b.len() > 1) {
        // Consecutive first digits and `b` goes on: its first digit alone fits.
        vec![b[0]]
    } else {
        let mut out = vec![DIGITS[da]];
        out.extend(midpoint(a.get(1..).unwrap_or(&[]), None));
        out
    }
}

fn increment_integer(x: &[u8]) -> Option<Vec<u8>> {
    let (head, digits) = (x[0], &x[1..]);
    let mut digits = digits.to_vec();
    for d in digits.iter_mut().rev() {
        if *d == LAST {
            *d = ZERO;
        } else {
            *d = DIGITS[index(*d) + 1];
            return Some([&[head][..], &digits].concat());
        }
    }
    // Every digit carried: move to the next head (one digit longer for
    // positive integers, one shorter for negative ones).
    match head {
        b'z' => None,
        b'Z' => Some(b"a0".to_vec()),
        _ => {
            let head = head + 1;
            if head > b'a' {
                digits.push(ZERO);
            } else {
                digits.pop();
            }
            Some([&[head][..], &digits].concat())
        }
    }
}

fn decrement_integer(x: &[u8]) -> Option<Vec<u8>> {
    let (head, digits) = (x[0], &x[1..]);
    let mut digits = digits.to_vec();
    for d in digits.iter_mut().rev() {
        if *d == ZERO {
            *d = LAST;
        } else {
            *d = DIGITS[index(*d) - 1];
            return Some([&[head][..], &digits].concat());
        }
    }
    match head {
        b'A' => None,
        b'a' => Some(vec![b'Z', LAST]),
        _ => {
            let head = head - 1;
            if head < b'Z' {
                digits.push(LAST);
            } else {
                digits.pop();
            }
            Some([&[head][..], &digits].concat())
        }
    }
}

/// The deterministic part of a key between `a` and `b`: strictly between
/// them and never a prefix of `b`.
fn base_between(a: Option<&[u8]>, b: Option<&[u8]>) -> Vec<u8> {
    let mut key = match (a, b) {
        (None, None) => b"a0".to_vec(),
        (None, Some(b)) => {
            let (ib, fb) = split(b);
            if ib == SMALLEST_INTEGER {
                [ib, &midpoint(&[], Some(fb))].concat()
            } else {
                decrement_integer(ib).expect("not the smallest integer")
            }
        }
        (Some(a), None) => {
            let (ia, fa) = split(a);
            increment_integer(ia).unwrap_or_else(|| [ia, &midpoint(fa, None)].concat())
        }
        (Some(a), Some(b)) => {
            let (ia, fa) = split(a);
            let (ib, fb) = split(b);
            if ia == ib {
                [ia, &midpoint(fa, Some(fb))].concat()
            } else {
                match increment_integer(ia) {
                    Some(i) if i.as_slice() < ib => i,
                    _ => [ia, &midpoint(fa, None)].concat(),
                }
            }
        }
    };
    // Keep going while the key is a prefix of `b`, so anything appended
    // still sorts before `b`.
    if let Some(b) = b {
        while let Some(rest) = b.strip_prefix(key.as_slice()) {
            let more = midpoint(&[], Some(rest));
            key.extend(more);
        }
    }
    key
}

/// `key` plus `SUFFIX_LEN` digits taken from `entropy` (last one non-zero).
fn with_suffix(mut key: Vec<u8>, mut entropy: u64) -> String {
    for i in 0..SUFFIX_LEN {
        let d = if i + 1 == SUFFIX_LEN { 1 + (entropy % (BASE as u64 - 1)) as usize } else { (entropy % BASE as u64) as usize };
        entropy /= BASE as u64;
        key.push(DIGITS[d]);
    }
    String::from_utf8(key).expect("ASCII")
}

/// 62 random bits from a v4 UUID (the same source as node ids, so it
/// builds wherever `uuid`'s random source does, wasm included).
fn entropy() -> u64 {
    (uuid::Uuid::new_v4().as_u128() as u64) & ((1 << 62) - 1)
}

fn check(a: Option<&str>, b: Option<&str>) {
    for key in [a, b].into_iter().flatten() {
        assert!(is_valid(key), "invalid rank key {key:?}");
    }
    if let (Some(a), Some(b)) = (a, b) {
        assert!(a < b, "rank keys out of order: {a:?} >= {b:?}");
    }
}

/// A new key strictly between `a` and `b`; `None` is the start (for `a`) or
/// the end (for `b`) of the list.
///
/// # Panics
///
/// If either key is not valid (`is_valid`) or `a >= b`.
pub fn between(a: Option<&str>, b: Option<&str>) -> String {
    between_with(a, b, entropy())
}

fn between_with(a: Option<&str>, b: Option<&str>, entropy: u64) -> String {
    check(a, b);
    with_suffix(base_between(a.map(str::as_bytes), b.map(str::as_bytes)), entropy)
}

/// `n` new keys in order, strictly between `a` and `b`, kept short and
/// evenly spread (for pastes and imports): runs at either end step the
/// integer part, runs in the middle are bisected.
///
/// # Panics
///
/// As `between`.
pub fn n_between(a: Option<&str>, b: Option<&str>, n: usize) -> Vec<String> {
    check(a, b);
    let mut out = Vec::with_capacity(n);
    fill(a, b, n, &mut out);
    out
}

fn fill(a: Option<&str>, b: Option<&str>, n: usize, out: &mut Vec<String>) {
    match n {
        0 => {}
        1 => out.push(between(a, b)),
        _ if b.is_none() => {
            let mut last = a.map(str::to_string);
            for _ in 0..n {
                let key = between(last.as_deref(), None);
                out.push(key.clone());
                last = Some(key);
            }
        }
        _ if a.is_none() => {
            let start = out.len();
            let mut first = b.map(str::to_string);
            for _ in 0..n {
                let key = between(None, first.as_deref());
                out.push(key.clone());
                first = Some(key);
            }
            out[start..].reverse();
        }
        _ => {
            let mid = n / 2;
            let c = between(a, b);
            fill(a, Some(&c), mid, out);
            out.push(c.clone());
            fill(Some(&c), b, n - mid - 1, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// xorshift64*: a seeded generator, so failures replay.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    fn assert_sorted(keys: &[String]) {
        for w in keys.windows(2) {
            assert!(w[0] < w[1], "{:?} !< {:?}", w[0], w[1]);
            assert!(w[0].as_bytes() < w[1].as_bytes());
        }
    }

    #[test]
    fn validity() {
        for key in ["a0", "a0V", "Zz", "b00", "a1x", "A000000000000000000000000001"] {
            assert!(is_valid(key), "{key}");
        }
        for key in ["", "a", "a00", "a1x0", "-a0", "a0!", "A00000000000000000000000000", "0a"] {
            assert!(!is_valid(key), "{key}");
        }
    }

    #[test]
    fn integer_steps() {
        assert_eq!(increment_integer(b"a0").unwrap(), b"a1");
        assert_eq!(increment_integer(b"az").unwrap(), b"b00");
        assert_eq!(increment_integer(b"Zz").unwrap(), b"a0");
        assert_eq!(increment_integer(b"Yzz").unwrap(), b"Z0");
        assert_eq!(decrement_integer(b"a0").unwrap(), b"Zz");
        assert_eq!(decrement_integer(b"b00").unwrap(), b"az");
        assert_eq!(decrement_integer(b"Z0").unwrap(), b"Yzz");
        assert_eq!(decrement_integer(SMALLEST_INTEGER), None);
        assert_eq!(increment_integer(b"zzzzzzzzzzzzzzzzzzzzzzzzzzz"), None);
    }

    #[test]
    fn simple_cases() {
        let first = between(None, None);
        assert!(first.starts_with("a0") && first.len() == 2 + SUFFIX_LEN);
        let after = between(Some(&first), None);
        assert!(after.starts_with("a1"));
        let before = between(None, Some(&first));
        assert!(before.starts_with("Zz"));
        let mid = between(Some(&first), Some(&after));
        assert_sorted(&[before, first, mid, after]);
    }

    /// The deterministic part is never a prefix of `b`, even when `b` is the
    /// deterministic part of an earlier key plus a suffix.
    #[test]
    fn suffix_never_passes_the_upper_bound() {
        for (a, b) in [(None, "a0V"), (Some("a0"), "a01"), (Some("a0x"), "a1y"), (Some("a0"), "a0101"), (Some("a0V"), "a0W1")] {
            for entropy in [0, u64::MAX >> 2, 12345] {
                let k = between_with(a, Some(b), entropy);
                assert!(is_valid(&k), "{k}");
                assert!(a.is_none_or(|a| a < k.as_str()) && k.as_str() < b, "{a:?} {k} {b}");
            }
        }
    }

    /// Property: random inserts anywhere keep every key valid, unique and in
    /// order (bytes, `str` and SQLite agree).
    #[test]
    fn random_inserts_stay_ordered() {
        for seed in 1..=20u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let mut keys: Vec<String> = Vec::new();
            for _ in 0..500 {
                let i = rng.below(keys.len() + 1);
                let a = i.checked_sub(1).map(|j| keys[j].as_str());
                let b = keys.get(i).map(String::as_str);
                let key = between_with(a, b, rng.next());
                assert!(is_valid(&key), "{key}");
                keys.insert(i, key);
            }
            assert_sorted(&keys);
            let mut sorted = keys.clone();
            sorted.sort_by(|x, y| x.as_bytes().cmp(y.as_bytes()));
            sorted.dedup();
            assert_eq!(sorted, keys);
        }
    }

    #[test]
    fn sqlite_binary_order_matches() {
        let mut rng = Rng(7);
        let mut keys: Vec<String> = Vec::new();
        for _ in 0..300 {
            let i = rng.below(keys.len() + 1);
            let key = between_with(i.checked_sub(1).map(|j| keys[j].as_str()), keys.get(i).map(String::as_str), rng.next());
            keys.insert(i, key);
        }
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (rank TEXT NOT NULL)").unwrap();
        let mut shuffled = keys.clone();
        for i in (1..shuffled.len()).rev() {
            shuffled.swap(i, rng.below(i + 1));
        }
        for key in &shuffled {
            conn.execute("INSERT INTO t VALUES (?1)", [key]).unwrap();
        }
        let mut stmt = conn.prepare("SELECT rank FROM t ORDER BY rank").unwrap();
        let ordered: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(ordered, keys);
        let less: i64 = conn.query_row("SELECT ?1 < ?2", [&keys[0], &keys[1]], |r| r.get(0)).unwrap();
        assert_eq!(less, 1);
    }

    #[test]
    fn appending_and_prepending_stay_short() {
        let mut last = between(None, None);
        let mut longest = 0;
        for _ in 0..10_000 {
            let key = between(Some(&last), None);
            assert!(key > last);
            longest = longest.max(key.len());
            last = key;
        }
        // `a0`..`az`, `b00`..`bzz`, then `c000`…: four integer characters.
        assert!(longest <= 4 + SUFFIX_LEN, "append grew to {longest}");

        let mut first = between(None, None);
        let mut longest = 0;
        for _ in 0..1_000 {
            let key = between(None, Some(&first));
            assert!(key < first);
            longest = longest.max(key.len());
            first = key;
        }
        // `Zz`..`Z0`, then `Yzz`…: three integer characters.
        assert!(longest <= 3 + SUFFIX_LEN, "prepend grew to {longest}");
    }

    /// The worst case: typing a list in the middle, one key after another
    /// before a fixed neighbour (or each before the last, after a fixed one).
    /// Every insert halves the gap, about one character per 4–5 inserts once
    /// the random suffixes are counted (measured: ~250 and ~210 characters
    /// after 1,000). Pastes and imports use `n_between` instead.
    #[test]
    fn inserting_next_to_the_previous_key_grows_linearly_but_slowly() {
        let a = between(None, None);
        let b = between(Some(&a), None);
        let mut last = a.clone();
        for _ in 0..1_000 {
            let key = between(Some(&last), Some(&b));
            assert!(last < key && key < b);
            last = key;
        }
        assert!(last.len() <= 320, "insert-after grew to {}", last.len());
        let mut first = b.clone();
        for _ in 0..1_000 {
            let key = between(Some(&a), Some(&first));
            assert!(a < key && key < first);
            first = key;
        }
        assert!(first.len() <= 280, "insert-before grew to {}", first.len());
    }

    #[test]
    fn n_between_is_short_and_ordered() {
        let keys = n_between(None, None, 10_000);
        assert_eq!(keys.len(), 10_000);
        assert_sorted(&keys);
        assert!(keys.iter().all(|k| k.len() <= 4 + SUFFIX_LEN));

        let a = between(None, None);
        let b = between(Some(&a), None);
        let keys = n_between(Some(&a), Some(&b), 1_000);
        assert_sorted(&keys);
        assert!(a < keys[0] && keys[999] < b);
        let longest = keys.iter().map(String::len).max().unwrap();
        assert!(longest <= 16, "bisected keys grew to {longest}");

        let keys = n_between(None, Some(&a), 1_000);
        assert_sorted(&keys);
        assert!(keys[999] < a);
        assert!(n_between(Some(&a), Some(&b), 0).is_empty());
    }

    /// Two replicas insert at the same place while offline: same neighbours,
    /// different keys, both still between the neighbours.
    #[test]
    fn concurrent_inserts_differ() {
        let a = between(None, None);
        let b = between(Some(&a), None);
        for (lo, hi) in [(None, None), (Some(a.as_str()), None), (None, Some(a.as_str())), (Some(a.as_str()), Some(b.as_str()))] {
            let mine = between_with(lo, hi, 1);
            let theirs = between_with(lo, hi, 2);
            assert_ne!(mine, theirs);
            for k in [&mine, &theirs] {
                assert!(lo.is_none_or(|lo| lo < k.as_str()) && hi.is_none_or(|hi| k.as_str() < hi));
            }
            // With real randomness too (collision odds 1 in 62^4 per pair).
            let keys: std::collections::HashSet<String> = (0..100).map(|_| between(lo, hi)).collect();
            assert!(keys.len() >= 99);
        }
    }

    #[test]
    #[should_panic(expected = "out of order")]
    fn rejects_reversed_bounds() {
        let a = between(None, None);
        let b = between(Some(&a), None);
        between(Some(&b), Some(&a));
    }
}
