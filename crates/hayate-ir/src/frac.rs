//! 兄弟要素の順序キー（fractional indexing、§DESIGN 6.10 / §8.1-4）。
//!
//! スライドや図形の並び順を `Vec` の添字でなく「0..1 の分数」を表す可変長キーで持つ。
//! 任意の2キーの間に新しいキーを生成できるため、挿入・並べ替えが他要素に影響せず、
//! Undo/将来のCRDT/Morph の同一性が安定する。
//!
//! 表現: `Vec<u8>` を「基数256の小数点以下の桁列」とみなす（例: `[128]` = 0.5）。
//! 生成結果は末尾が必ず非ゼロになるため、`Vec<u8>` の辞書式順序がそのまま分数順序に一致する。

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FracIndex(pub Vec<u8>);

const BASE: u16 = 256;

impl FracIndex {
    /// `lo` と `hi` の中間キーを生成する。`None` は開放端（lo=0.0 / hi=1.0）。
    /// 事前条件: `lo < hi`（指定時）。返り値 `m` は `lo < m < hi` を満たす。
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
                // l と h の間に整数桁の余地がある → 中点を採用して終了（必ず >= 1 で末尾非ゼロ）
                out.push(((l + h) / 2) as u8);
                return FracIndex(out);
            }
            // 桁が等しい or 隣接 → lo の桁を採用して次の桁へ降りる
            out.push(l as u8);
            if l < h {
                // 隣接(l+1==h): 以降は上限が開放(1.0)になる
                hi = None;
            }
            i += 1;
        }
    }

    /// 末尾に追加する新規キー（既存最大 `last` の後ろ）。
    pub fn after(last: Option<&FracIndex>) -> FracIndex {
        FracIndex::between(last, None)
    }

    /// 先頭に追加する新規キー（既存最小 `first` の前）。
    pub fn before(first: Option<&FracIndex>) -> FracIndex {
        FracIndex::between(None, first)
    }
}

#[cfg(test)]
mod tests {
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
        // 同じ2点の間に100回挿入しても順序が壊れないこと
        let mut lo = FracIndex::between(None, None);
        let hi = FracIndex::after(Some(&lo));
        let mut prev = lo.clone();
        for _ in 0..100 {
            let m = FracIndex::between(Some(&lo), Some(&hi));
            assert!(lo < m && m < hi, "lo={lo:?} m={m:?} hi={hi:?}");
            assert!(m > prev || prev == lo, "monotonic shrink toward lo");
            prev = m.clone();
            lo = m; // 毎回 lo 寄りに詰めていく
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
        assert_eq!(keys, sorted, "append順とソート順が一致すること");
    }
}
