use std::fmt;

/// Text that is none of a setting's names, as their `Display` writes them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{text:?} is not a {what}; one of: {expected}")]
pub struct ParseNameError {
    what: &'static str,
    text: String,
    expected: String,
}

/// The value in `all` whose `Display` is `text`, so parsing reads back
/// exactly what displaying writes. `what` names the setting in the error,
/// which lists every name.
pub fn parse_name<T: Copy + fmt::Display>(
    what: &'static str,
    all: &[T],
    text: &str,
) -> Result<T, ParseNameError> {
    all.iter()
        .find(|value| value.to_string() == text)
        .copied()
        .ok_or_else(|| ParseNameError {
            what,
            text: text.to_owned(),
            expected: all
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
        })
}
