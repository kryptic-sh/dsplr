//! `dsplr backend`: what detection checked, which backend it found, and what
//! that backend can do.

use dsplr_backend::{Check, Detection};
use dsplr_model::Capabilities;

/// The text `dsplr backend` prints.
pub fn describe(detection: &Detection) -> String {
    let found = match &detection.backend {
        Some(backend) => format!(
            "backend: {}\ncapabilities: {}\n",
            backend.name(),
            capability_names(&backend.capabilities()).join(", ")
        ),
        None => "backend: none found\n".to_owned(),
    };
    describe_checks(&detection.checks) + &found
}

/// One line per check, in the order detection made them.
pub fn describe_checks(checks: &[Check]) -> String {
    checks
        .iter()
        .map(|check| format!("{}: {}\n", check.what, check.found))
        .collect()
}

/// The names of the capabilities a backend has, or `none (read-only)`.
fn capability_names(capabilities: &Capabilities) -> Vec<String> {
    let Capabilities {
        custom_modes,
        mirroring,
        adaptive_sync,
        fractional_scale,
        per_output_scale,
        test_before_apply,
        gaps_allowed,
        native_persistence,
        saves_on_apply,
        primary,
        colour_preset,
        bit_depth,
        overscan,
        rgb_range,
    } = capabilities;
    let flags = [
        (custom_modes, "custom modes"),
        (mirroring, "mirroring"),
        (fractional_scale, "fractional scale"),
        (per_output_scale, "per-output scale"),
        (test_before_apply, "test before apply"),
        (gaps_allowed, "gaps allowed"),
        (native_persistence, "native persistence"),
        (saves_on_apply, "saves on every apply"),
        (primary, "primary output"),
        (colour_preset, "colour preset"),
        (bit_depth, "bit depth"),
        (overscan, "overscan"),
        (rgb_range, "RGB range"),
    ];
    let mut names: Vec<String> = flags
        .into_iter()
        .filter(|(on, _)| **on)
        .map(|(_, name)| name.to_owned())
        .collect();
    if !adaptive_sync.is_empty() {
        let values: Vec<_> = adaptive_sync
            .iter()
            .map(|value| crate::list::adaptive_sync_name(*value))
            .collect();
        names.push(format!("adaptive sync ({})", values.join(", ")));
    }
    if names.is_empty() {
        names.push("none (read-only)".to_owned());
    }
    names
}

#[cfg(test)]
mod tests {
    use dsplr_backend::{Check, Detection};
    use dsplr_model::{AdaptiveSync, Capabilities};

    use super::{capability_names, describe};

    #[test]
    fn no_backend_lists_the_checks_and_says_so() {
        let detection = Detection {
            checks: vec![Check {
                what: "platform",
                found: "not Linux: dsplr supports Linux only".into(),
            }],
            backend: None,
        };
        assert_eq!(
            describe(&detection),
            "platform: not Linux: dsplr supports Linux only\nbackend: none found\n"
        );
    }

    #[test]
    fn a_backend_with_nothing_optional_is_read_only() {
        assert_eq!(
            capability_names(&Capabilities::default()),
            ["none (read-only)"]
        );
    }

    #[test]
    fn capabilities_are_named_in_order() {
        let capabilities = Capabilities {
            mirroring: true,
            gaps_allowed: true,
            adaptive_sync: vec![AdaptiveSync::Off, AdaptiveSync::On],
            ..Capabilities::default()
        };
        assert_eq!(
            capability_names(&capabilities),
            ["mirroring", "gaps allowed", "adaptive sync (off, on)"]
        );
    }
}
