//! Unit tests for the parent module.

use super::*;

#[test]
fn open_ends() {
    let a = FracIndex::between(None, None);
    let b = FracIndex::after(Some(&a));
    let c = FracIndex::before(Some(&a));
    assert!(c < a, "{c:?} < {a:?}");
    assert!(a < b, "{a:?} < {b:?}");
}

#[test]
fn strictly_between() {
    let lo = FracIndex(vec![128]);
    let hi = FracIndex(vec![129]);
    let m = FracIndex::between(Some(&lo), Some(&hi));
    assert!(lo < m && m < hi, "lo={lo:?} m={m:?} hi={hi:?}");
}

#[test]
fn repeated_subdivision_stays_ordered() {
    // Inserting 100 times between the same two points must not break ordering.
    let mut lo = FracIndex::between(None, None);
    let hi = FracIndex::after(Some(&lo));
    let mut prev = lo.clone();
    for _ in 0..100 {
        let m = FracIndex::between(Some(&lo), Some(&hi));
        assert!(lo < m && m < hi, "lo={lo:?} m={m:?} hi={hi:?}");
        assert!(m > prev || prev == lo, "monotonic shrink toward lo");
        prev = m.clone();
        lo = m; // Keep packing toward lo each iteration.
    }
}

#[test]
fn sequence_of_appends_is_increasing() {
    let mut keys = Vec::new();
    let mut last: Option<FracIndex> = None;
    for _ in 0..50 {
        let k = FracIndex::after(last.as_ref());
        if let Some(p) = &last {
            assert!(p < &k);
        }
        last = Some(k.clone());
        keys.push(k);
    }
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "append order matches sorted order");
}
