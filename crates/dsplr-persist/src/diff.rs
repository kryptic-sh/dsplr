//! A line diff, to show what saving would change in a file someone else
//! wrote or edited before dsplr replaces it.

/// The most cells the comparison table may have. Past it, the lines that
/// differ are shown as all removed, then all added: still a true account of
/// the change, only not the shortest one.
const MAX_CELLS: usize = 1 << 20;

/// `old` turned into `new`, line by line: `-` before a line only in `old`,
/// `+` before one only in `new`, and a space before one in both. Every line
/// is shown. Lines are compared without their line endings, so a missing
/// final newline is not a difference.
///
/// The lines in both are a longest common subsequence of the two, found by
/// the textbook dynamic programme (Cormen et al., _Introduction to
/// Algorithms_, 15.4) after the common first and last lines are set aside.
pub fn line_diff(old: &str, new: &str) -> String {
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    let prefix = old
        .iter()
        .zip(&new)
        .take_while(|(old, new)| old == new)
        .count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(old, new)| old == new)
        .count();
    let (removed, added) = (
        &old[prefix..old.len() - suffix],
        &new[prefix..new.len() - suffix],
    );
    let mut text = String::new();
    let mut line = |marker: char, line: &str| {
        text.push(marker);
        text.push_str(line);
        text.push('\n');
    };
    for same in &old[..prefix] {
        line(' ', same);
    }
    let cells = (removed.len() + 1).saturating_mul(added.len() + 1);
    if cells > MAX_CELLS {
        removed.iter().for_each(|gone| line('-', gone));
        added.iter().for_each(|came| line('+', came));
    } else {
        // longest[i][j]: the length of a longest common subsequence of
        // removed[i..] and added[j..].
        let width = added.len() + 1;
        let mut longest = vec![0_usize; (removed.len() + 1) * width];
        for i in (0..removed.len()).rev() {
            for j in (0..added.len()).rev() {
                longest[i * width + j] = if removed[i] == added[j] {
                    longest[(i + 1) * width + j + 1] + 1
                } else {
                    longest[(i + 1) * width + j].max(longest[i * width + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < removed.len() && j < added.len() {
            if removed[i] == added[j] {
                line(' ', removed[i]);
                (i, j) = (i + 1, j + 1);
            } else if longest[(i + 1) * width + j] >= longest[i * width + j + 1] {
                line('-', removed[i]);
                i += 1;
            } else {
                line('+', added[j]);
                j += 1;
            }
        }
        removed[i..].iter().for_each(|gone| line('-', gone));
        added[j..].iter().for_each(|came| line('+', came));
    }
    for same in &old[old.len() - suffix..] {
        line(' ', same);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::line_diff;

    fn kept(diff: &str) -> usize {
        diff.lines().filter(|line| line.starts_with(' ')).count()
    }

    #[test]
    fn a_change_shows_what_went_and_what_came() {
        assert_eq!(line_diff("a\nb\nc\n", "a\nc\nd\n"), " a\n-b\n c\n+d\n");
        assert_eq!(line_diff("same\n", "same"), " same\n");
        assert_eq!(line_diff("", "new\n"), "+new\n");
        assert_eq!(line_diff("old\n", ""), "-old\n");
        assert_eq!(line_diff("", ""), "");
    }

    /// Cormen et al.'s example (figure 15.8): ABCBDAB and BDCABA have a
    /// longest common subsequence of four.
    #[test]
    fn the_lines_kept_are_a_longest_common_subsequence() {
        let lines = |text: &str| text.chars().map(|c| format!("{c}\n")).collect::<String>();
        let diff = line_diff(&lines("ABCBDAB"), &lines("BDCABA"));
        assert_eq!(kept(&diff), 4, "{diff}");
        let removed = diff.lines().filter(|line| line.starts_with('-')).count();
        let added = diff.lines().filter(|line| line.starts_with('+')).count();
        assert_eq!((removed, added), (3, 2), "{diff}");
        // Read back, each side is the text it came from.
        let side = |skip: char| -> String {
            diff.lines()
                .filter(|line| !line.starts_with(skip))
                .map(|line| &line[1..])
                .collect()
        };
        assert_eq!(side('+'), "ABCBDAB");
        assert_eq!(side('-'), "BDCABA");
    }

    #[test]
    fn past_the_table_s_limit_the_middle_is_replaced_whole() {
        let old: String = (0..1100).map(|n| format!("old {n}\n")).collect();
        let new: String = (0..1100).map(|n| format!("new {n}\n")).collect();
        let text = format!("first\n{old}last\n");
        let diff = line_diff(&text, &format!("first\n{new}last\n"));
        let lines: Vec<&str> = diff.lines().collect();
        assert_eq!(lines.first(), Some(&" first"));
        assert_eq!(lines.last(), Some(&" last"));
        assert_eq!(lines[1], "-old 0");
        assert_eq!(lines[1101], "+new 0");
        assert_eq!(lines.len(), 2 + 2 * 1100);
    }
}
