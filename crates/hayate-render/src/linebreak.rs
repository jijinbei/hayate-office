//! Line-breaking policy layer (DESIGN 6.16). This is a swappable core policy: the caller
//! measures each grapheme's pixel advance, and this layer only decides where lines break.
//! gpui-free and unit-testable.

/// One grapheme/char with its measured pixel advance. Measurement is done elsewhere by the
/// caller; this layer only decides break points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Item {
    pub ch: char,
    pub advance: f32,
}

/// A line-breaking policy. Implementations decide where lines start given measured items and
/// a maximum line width in pixels.
pub trait LineBreaker {
    /// Returns the start index of each line (always includes 0).
    fn line_starts(&self, items: &[Item], max_width: f32) -> Vec<usize>;
}

/// Characters prohibited at the start of a line (gyoutou kinsoku): closing punctuation,
/// closing brackets, small kana, prolonged sound mark, iteration mark, etc.
const LINE_START_PROHIBITED: &[char] = &[
    '。', '、', '，', '．', '）', '」', '』', '】', '〕', '｝', '！', '？', '・', 'ー', 'ぁ',
    'ぃ', 'ぅ', 'ぇ', 'ぉ', 'っ', 'ゃ', 'ゅ', 'ょ', '々',
];

/// Characters prohibited at the end of a line (gyoumatsu kinsoku): opening brackets that must
/// not be separated from what follows.
const LINE_END_PROHIBITED: &[char] = &['（', '「', '『', '【', '〔', '｛'];

/// Whether `ch` is prohibited from starting a line.
pub fn is_line_start_prohibited(ch: char) -> bool {
    LINE_START_PROHIBITED.contains(&ch)
}

/// Whether `ch` is prohibited from ending a line.
pub fn is_line_end_prohibited(ch: char) -> bool {
    LINE_END_PROHIBITED.contains(&ch)
}

/// Greedy width-based wrapping. Accumulate advances; when adding an item would exceed
/// `max_width`, start a new line. Prefers breaking at the last ASCII space before the
/// overflow if there is one on the current line; otherwise hard-breaks.
pub struct DefaultBreaker;

impl LineBreaker for DefaultBreaker {
    fn line_starts(&self, items: &[Item], max_width: f32) -> Vec<usize> {
        greedy_line_starts(items, max_width, |_items, candidate, _line_start| candidate)
    }
}

/// Same greedy width logic as `DefaultBreaker`, but enforces Japanese kinsoku rules:
/// no line may start with a line-start-prohibited char, and no line may end with a
/// line-end-prohibited char. Between two CJK ideographs/kana a break is allowed by default.
pub struct JapaneseBreaker;

impl LineBreaker for JapaneseBreaker {
    fn line_starts(&self, items: &[Item], max_width: f32) -> Vec<usize> {
        greedy_line_starts(items, max_width, adjust_for_kinsoku)
    }
}

/// Shared greedy wrapping driver. `adjust` is given the proposed break index (the start of the
/// next line) and may move it earlier to satisfy a policy; it must return an index strictly
/// greater than `line_start` to guarantee forward progress.
fn greedy_line_starts(
    items: &[Item],
    max_width: f32,
    adjust: impl Fn(&[Item], usize, usize) -> usize,
) -> Vec<usize> {
    let mut starts = vec![0usize];
    if items.is_empty() {
        return starts;
    }

    let mut line_start = 0usize;
    while line_start < items.len() {
        // Find the break index: the first index where the accumulated advance from
        // `line_start` exceeds `max_width`. `break_idx` is the start of the next line.
        let mut width = 0.0f32;
        let mut break_idx = items.len();
        let mut last_space: Option<usize> = None; // index *after* the space

        for i in line_start..items.len() {
            let next = width + items[i].advance;
            if next > max_width && i > line_start {
                // Adding this item overflows and we have at least one item on the line.
                break_idx = i;
                break;
            }
            width = next;
            if items[i].ch == ' ' {
                last_space = Some(i + 1);
            }
        }

        if break_idx >= items.len() {
            // Everything from line_start fits on this line.
            break;
        }

        // Prefer breaking at the last ASCII space before the overflow, if one exists on the
        // current line (and it makes progress).
        let mut candidate = break_idx;
        if let Some(sp) = last_space {
            if sp > line_start && sp <= break_idx {
                candidate = sp;
            }
        }

        // Let the policy adjust the break (e.g. kinsoku). Guarantee forward progress.
        candidate = adjust(items, candidate, line_start);
        if candidate <= line_start {
            candidate = line_start + 1;
        }
        if candidate >= items.len() {
            break;
        }

        starts.push(candidate);
        line_start = candidate;
    }

    starts
}

/// Adjust a proposed break index so that the next line does not start with a prohibited char
/// and the current line does not end with a prohibited char. Moves the break earlier as
/// needed, never past `line_start + 1` (which guarantees progress).
fn adjust_for_kinsoku(items: &[Item], candidate: usize, line_start: usize) -> usize {
    let mut idx = candidate;

    // Move earlier while the next line would start with a prohibited char, or the current
    // line would end with a prohibited char. Stop at line_start + 1 to keep progress.
    while idx > line_start + 1 {
        let starts_bad = idx < items.len() && is_line_start_prohibited(items[idx].ch);
        let ends_bad = idx >= 1 && is_line_end_prohibited(items[idx - 1].ch);
        if starts_bad || ends_bad {
            idx -= 1;
        } else {
            break;
        }
    }

    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(s: &str, advance: f32) -> Vec<Item> {
        s.chars().map(|ch| Item { ch, advance }).collect()
    }

    /// Reconstruct each line's text from the line-start indices for easy assertions.
    fn lines(items: &[Item], starts: &[usize]) -> Vec<String> {
        let mut out = Vec::new();
        for (i, &s) in starts.iter().enumerate() {
            let end = starts.get(i + 1).copied().unwrap_or(items.len());
            out.push(items[s..end].iter().map(|it| it.ch).collect());
        }
        out
    }

    #[test]
    fn default_breaker_wraps_at_spaces() {
        // "aaaa bbbb cccc": each char advance = 10, so each 4-char word is 40px,
        // plus a 10px space. At max_width = 90 only "aaaa bbbb" (90px incl. trailing
        // space candidate) fits per attempt; it should break at spaces.
        let its = items("aaaa bbbb cccc", 10.0);
        let starts = DefaultBreaker.line_starts(&its, 90.0);
        let ls = lines(&its, &starts);

        assert!(starts.len() > 1, "expected multiple lines, got {:?}", ls);
        assert_eq!(starts[0], 0);
        // No line should start with a space (breaks happen at spaces, consuming them at
        // line end).
        for line in &ls {
            assert!(
                !line.starts_with(' '),
                "line unexpectedly starts with space: {:?}",
                ls
            );
        }
        // Each produced word fragment should be a whole word, not split mid-word.
        let joined: String = ls.join("|");
        assert!(
            joined.contains("aaaa") && joined.contains("bbbb") && joined.contains("cccc"),
            "words should stay intact: {:?}",
            ls
        );
    }

    #[test]
    fn japanese_breaker_never_starts_with_prohibited() {
        // "これは。" with width forcing a break right before 。 — kinsoku must pull 。 up.
        let its = items("これは。", 10.0);
        // max_width = 30 fits exactly 3 chars; a naive break would put 。 at the next line
        // start.
        let starts = JapaneseBreaker.line_starts(&its, 30.0);
        let ls = lines(&its, &starts);

        for (i, &s) in starts.iter().enumerate() {
            if i == 0 {
                continue;
            }
            assert!(
                !is_line_start_prohibited(its[s].ch),
                "line {} starts with prohibited char {:?}: {:?}",
                i,
                its[s].ch,
                ls
            );
        }
    }

    #[test]
    fn japanese_breaker_never_ends_with_prohibited() {
        // "あい「うえお": with a width that would break right after 「, kinsoku must pull
        // 「 down to the next line so it does not end a line.
        let its = items("あい「うえお", 10.0);
        let starts = JapaneseBreaker.line_starts(&its, 30.0);
        let ls = lines(&its, &starts);

        for i in 0..starts.len() {
            let end = starts.get(i + 1).copied().unwrap_or(its.len());
            let last = its[end - 1].ch;
            assert!(
                !is_line_end_prohibited(last),
                "line {} ends with prohibited char {:?}: {:?}",
                i,
                last,
                ls
            );
        }
    }

    #[test]
    fn single_item_wider_than_max_makes_progress() {
        // A single 200px char with max_width = 50 must still advance one item per line and
        // not loop forever.
        let its = items("WWW", 200.0);
        let starts = JapaneseBreaker.line_starts(&its, 50.0);
        assert_eq!(starts, vec![0, 1, 2]);

        let starts_def = DefaultBreaker.line_starts(&its, 50.0);
        assert_eq!(starts_def, vec![0, 1, 2]);
    }
}
