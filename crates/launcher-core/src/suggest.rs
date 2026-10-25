//! "Did you mean ...?" for unknown names: the closest candidate by edit distance, if it is close.

/// Edit distance, case-insensitive, counting a swap of two neighbouring letters as one edit
/// (optimal string alignment): "titel" is one edit from "title".
pub fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.to_lowercase().chars().collect();
    let b: Vec<char> = b.to_lowercase().chars().collect();
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, v) in d[0].iter_mut().enumerate() {
        *v = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[n][m]
}

/// The closest candidate within a third of the word's length (at least 1, at most 3 edits).
pub fn did_you_mean<'a>(
    word: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Option<&'a str> {
    let limit = (word.chars().count() / 3).clamp(1, 3);
    candidates
        .into_iter()
        .map(|c| (distance(word, c), c))
        .filter(|(d, _)| *d <= limit)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

/// `a`, `b` or `c`, each in backticks.
pub fn one_of(words: &[&str]) -> String {
    let quoted: Vec<String> = words.iter().map(|w| format!("`{w}`")).collect();
    match quoted.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_names_are_suggested() {
        assert_eq!(
            did_you_mean("dropdwn", ["dropdown", "checkbox"]),
            Some("dropdown")
        );
        assert_eq!(did_you_mean("xyz", ["dropdown", "checkbox"]), None);
        assert_eq!(did_you_mean("titel", ["title", "theme"]), Some("title"));
        assert_eq!(did_you_mean("mdoe", ["mode"]), Some("mode"));
    }
}
