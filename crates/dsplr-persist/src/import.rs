//! What importing another tool's profiles shares, whichever tool:
//! [`crate::kanshi`] and [`crate::autorandr`].

use std::str::FromStr;

use dsplr_model::Layout;

use crate::profile::ProfileName;

/// A kanshi profile as a dsplr profile.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub name: ProfileName,
    /// The kanshi profile it came from: its name, or kanshi's for an
    /// anonymous one, and where it is.
    pub from: String,
    pub layout: Layout,
    /// What was not taken from kanshi as written, one note each.
    pub notes: Vec<String>,
}

/// A kanshi profile not imported, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub from: String,
    pub reason: String,
}

/// The dsplr profile name for another tool's profile `name`: `name` itself,
/// when it is a profile name; else one made from it, its unusable
/// characters each `_`, which is noted.
pub fn name_from(name: &str, notes: &mut Vec<String>) -> Result<ProfileName, String> {
    if let Ok(name) = ProfileName::from_str(name) {
        return Ok(name);
    }
    let made: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let made = made.replace("..", "_.");
    let made = made.trim_end_matches([' ', '.']);
    let made = if made.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
        made.to_owned()
    } else {
        format!("_{made}")
    };
    let made = ProfileName::from_str(&made)
        .map_err(|error| format!("its name \"{name}\" cannot be a dsplr profile's: {error}"))?;
    notes.push(format!(
        "named \"{made}\" here: \"{name}\" is not a dsplr profile name"
    ));
    Ok(made)
}
