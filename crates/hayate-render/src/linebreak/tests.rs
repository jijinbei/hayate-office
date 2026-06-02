//! Unit tests for the parent module.

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
