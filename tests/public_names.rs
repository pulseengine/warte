//! warte is a public template: it never names the owners' private
//! configuration repo or their own hosts (REQ-TEMPLATE-001). The names are
//! kept here only as SHA-256 hashes, so this check does not name them either.
//! Existing requirement IDs that hold a name (`REQ-X-001` style) are allowed.

use sha2::{Digest, Sha256};

/// SHA-256 of each forbidden lowercase word, or of two words joined by a dot.
const FORBIDDEN: &[&str] = &[
    "4d50c52025f8bc9d06fe9f5ca6470e9433baa8509e127b059b6b286f8b4dc71d",
    "7e980bf821d7b1641cf57b3d58c8d387551f0a40d4ac1ff931f8b9550068543b",
    "d64c15b32c180e6ba0bb6aa1122ecb892f2d832684e57a882d1aaf297919fb5a",
];

fn sha(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

/// Every `line:column` in `text` where a word (or `word.word`) hashes to one
/// of `forbidden`, except the middle part of an uppercase ID like `REQ-X-001`.
pub fn violations(text: &str, forbidden: &[&str]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (ln, line) in text.lines().enumerate() {
        // ASCII alphanumeric runs: byte offsets stay on char boundaries
        let mut words: Vec<(usize, &str)> = Vec::new();
        let mut start: Option<usize> = None;
        for (i, c) in line
            .char_indices()
            .chain(std::iter::once((line.len(), ' ')))
        {
            match (c.is_ascii_alphanumeric(), start) {
                (true, None) => start = Some(i),
                (false, Some(s)) => {
                    words.push((s, &line[s..i]));
                    start = None;
                }
                _ => {}
            }
        }
        let id_part = |i: usize, w: &str| {
            let before = &line[..i];
            let after = &line[i + w.len()..];
            w.chars().all(|c| c.is_ascii_uppercase())
                && before.ends_with('-')
                && before[..before.len() - 1]
                    .chars()
                    .rev()
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .count()
                    > 0
                && after.starts_with('-')
                && after[1..].starts_with(|c: char| c.is_ascii_digit())
        };
        for (k, &(i, w)) in words.iter().enumerate() {
            let lw = w.to_ascii_lowercase();
            let mut hit = forbidden.contains(&sha(&lw).as_str()) && !id_part(i, w);
            if let Some(&(j, n)) = words.get(k + 1) {
                if j == i + w.len() + 1 && line.as_bytes()[i + w.len()] == b'.' {
                    let pair = format!("{lw}.{}", n.to_ascii_lowercase());
                    hit |= forbidden.contains(&sha(&pair).as_str());
                }
            }
            if hit {
                out.push((ln + 1, i + 1));
            }
        }
    }
    out
}

// rivet: verifies REQ-TEMPLATE-001
#[test]
fn the_check_catches_a_planted_name_and_spares_ids() {
    let planted = sha("zebrafoo");
    let pair = sha("zebra.lan");
    let f = [planted.as_str(), pair.as_str()];
    assert_eq!(violations("deployed by zebrafoo today", &f), vec![(1, 13)]);
    assert_eq!(
        violations("x\nZebraFoo agent", &f),
        vec![(2, 1)],
        "any case"
    );
    assert_eq!(
        violations("ends with zebrafoo", &f),
        vec![(1, 11)],
        "last word"
    );
    assert_eq!(
        violations("host mac.zebra.lan", &f),
        vec![(1, 10)],
        "a dotted name"
    );
    assert!(
        violations("see REQ-ZEBRAFOO-001 and DD-ZEBRAFOO-12", &f).is_empty(),
        "IDs"
    );
    assert_eq!(violations("REQ-ZEBRAFOO-x", &f).len(), 1, "not an ID");
    assert_eq!(
        violations("the-zebrafoo-1", &f).len(),
        1,
        "lowercase is not an ID"
    );
    assert!(violations("zebrafool zebra lan", &f).is_empty());
}

// rivet: verifies REQ-TEMPLATE-001
#[test]
fn no_tracked_file_names_a_private_repo_or_host() {
    let out = std::process::Command::new("git")
        .args(["ls-files", "-z"])
        .output()
        .expect("git");
    assert!(out.status.success(), "git ls-files failed");
    let files: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|f| !f.is_empty())
        .map(String::from)
        .collect();
    let mut scanned = 0;
    let mut bad = Vec::new();
    for f in &files {
        let Ok(text) = std::fs::read_to_string(f) else {
            continue; // binary, or removed in the working tree
        };
        scanned += 1;
        for (l, c) in violations(&text, FORBIDDEN) {
            bad.push(format!("{f}:{l}:{c}"));
        }
    }
    // fail closed: an empty scan proves nothing
    assert!(scanned > 100, "only {scanned} files scanned");
    assert!(
        bad.is_empty(),
        "a private name is back (see tests/public_names.rs): {bad:?}"
    );
}
