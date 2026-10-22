//! Word-level diffs for onion skinning.

use serde::Serialize;
use similar::{Algorithm, ChangeTag, TextDiff};

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Op {
    /// "equal" | "insert" | "delete"
    pub tag: &'static str,
    pub text: String,
}

/// Diff `old` -> `new` by words. Concatenating the equal+insert ops rebuilds
/// `new` exactly, and equal+delete rebuilds `old`, so the frontend can map
/// ops onto editor positions by counting lengths.
pub fn words(old: &str, new: &str) -> Vec<Op> {
    let diff = TextDiff::configure().algorithm(Algorithm::Patience).diff_words(old, new);
    let raw: Vec<(ChangeTag, &str)> = diff.iter_all_changes().map(|c| (c.tag(), c.value())).collect();

    // Fold the single spaces between changed words into the change, so
    // "a b" -> "x y" reads as one replacement instead of a stripe of fragments.
    let mut out: Vec<Op> = vec![];
    let (mut del, mut ins) = (String::new(), String::new());
    let flush = |out: &mut Vec<Op>, del: &mut String, ins: &mut String| {
        if !del.is_empty() {
            out.push(Op { tag: "delete", text: std::mem::take(del) });
        }
        if !ins.is_empty() {
            out.push(Op { tag: "insert", text: std::mem::take(ins) });
        }
    };
    for (i, &(tag, text)) in raw.iter().enumerate() {
        match tag {
            ChangeTag::Delete => del.push_str(text),
            ChangeTag::Insert => ins.push_str(text),
            ChangeTag::Equal => {
                let in_change = !del.is_empty() || !ins.is_empty();
                let next_changes = raw.get(i + 1).is_some_and(|(t, _)| *t != ChangeTag::Equal);
                if in_change && next_changes && text.trim().is_empty() {
                    del.push_str(text);
                    ins.push_str(text);
                } else {
                    flush(&mut out, &mut del, &mut ins);
                    match out.last_mut() {
                        Some(last) if last.tag == "equal" => last.text.push_str(text),
                        _ => out.push(Op { tag: "equal", text: text.to_string() }),
                    }
                }
            }
        }
    }
    flush(&mut out, &mut del, &mut ins);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rebuild(ops: &[Op], skip: &str) -> String {
        ops.iter().filter(|o| o.tag != skip).map(|o| o.text.as_str()).collect()
    }

    #[test]
    fn ops_rebuild_both_sides() {
        let old = "You stop at the crosswalk on Alma Street with eleven people.";
        let new = "You stop at the corner of Alma Street with eleven other people.";
        let ops = words(old, new);
        assert_eq!(rebuild(&ops, "delete"), new);
        assert_eq!(rebuild(&ops, "insert"), old);
    }

    #[test]
    fn adjacent_word_changes_merge() {
        let ops = words("the red big dog", "the blue small dog");
        let changes: Vec<_> = ops.iter().filter(|o| o.tag != "equal").collect();
        assert_eq!(changes.len(), 2, "{ops:?}");
    }
}
