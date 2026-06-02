//! Sibling order key (fractional indexing, DESIGN 6.10 / 8.1-4).
//!
//! Slide and shape order is stored as a variable-length key representing a fraction in
//! 0..1, rather than a `Vec` index. A new key can always be generated between any two
//! existing keys, so insertion/reordering does not disturb other elements, which keeps
//! Undo / future CRDT / Morph identity stable.
//!
//! Representation: `Vec<u8>` is treated as the digits after the radix point in base 256
//! (e.g. `[128]` = 0.5). Generated results never end in a zero byte, so the lexicographic
//! ordering of `Vec<u8>` matches the fraction ordering exactly.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FracIndex(pub Vec<u8>);

const BASE: u16 = 256;

impl FracIndex {
    /// Generate a key between `lo` and `hi`. `None` is an open end (lo = 0.0 / hi = 1.0).
    /// Precondition: `lo < hi` (when both given). The result `m` satisfies `lo < m < hi`.
    pub fn between(lo: Option<&FracIndex>, hi: Option<&FracIndex>) -> FracIndex {
        let lo = lo.map(|x| x.0.as_slice()).unwrap_or(&[]);
        let mut hi: Option<&[u8]> = hi.map(|x| x.0.as_slice());
        let mut out = Vec::new();
        let mut i = 0usize;
        loop {
            let l = lo.get(i).copied().unwrap_or(0) as u16;
            let h = match hi {
                Some(h) => h.get(i).copied().unwrap_or(0) as u16,
                None => BASE, // 1.0
            };
            if l + 1 < h {
                // There is room for an integer digit between l and h: take the midpoint and
                // stop (always >= 1, so the last byte is non-zero).
                out.push(((l + h) / 2) as u8);
                return FracIndex(out);
            }
            // Digits equal or adjacent: take lo's digit and descend to the next one.
            out.push(l as u8);
            if l < h {
                // Adjacent (l+1 == h): the upper bound becomes open (1.0) from here on.
                hi = None;
            }
            i += 1;
        }
    }

    /// New key appended after the current maximum `last`.
    pub fn after(last: Option<&FracIndex>) -> FracIndex {
        FracIndex::between(last, None)
    }

    /// New key prepended before the current minimum `first`.
    pub fn before(first: Option<&FracIndex>) -> FracIndex {
        FracIndex::between(None, first)
    }
}

#[cfg(test)]
mod tests;
