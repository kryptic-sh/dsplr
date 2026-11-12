//! The system's PNP vendor names: hwdata's `pnp.ids`, read at runtime for
//! [`PnpNames`] (owner's call, 2026-11-13), so that dsplr names a monitor's
//! make as wlroots and Hyprland do without carrying the table itself.

use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use dsplr_model::PnpNames;

/// Where hwdata installs `pnp.ids`, and where libdisplay-info 0.3.0's
/// build looks for it when pkg-config names no hwdata directory.
pub const PNP_IDS: &str = "/usr/share/hwdata/pnp.ids";

/// The names in [`PNP_IDS`], read once per process ([`read_names`]).
pub fn system_names() -> &'static PnpNames {
    static NAMES: OnceLock<PnpNames> = OnceLock::new();
    NAMES.get_or_init(|| read_names(Path::new(PNP_IDS)))
}

/// The names in the `pnp.ids` at `path`. A file that cannot be read gives
/// no table, as one not installed does: makes then stay PNP codes, which
/// still match another desktop's codes but not wlroots' or Hyprland's
/// names. Text that is not UTF-8 is read lossily, a bad byte in one name
/// spoiling only that name.
pub fn read_names(path: &Path) -> PnpNames {
    fs::read(path).map_or_else(
        |_| PnpNames::default(),
        |bytes| PnpNames::parse(&String::from_utf8_lossy(&bytes)),
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use dsplr_model::PnpNames;

    use super::read_names;

    #[test]
    fn the_file_s_names_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pnp.ids");
        fs::write(&path, "DEL\tDell Inc.\nAUS\tASUSTek COMPUTER INC\n").unwrap();
        let names = read_names(&path);
        assert_eq!(names.make("DEL"), "Dell Inc.");
        assert_eq!(names.make("XYZ"), "PNP(XYZ)");
    }

    #[test]
    fn no_file_is_no_table() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_names(&dir.path().join("pnp.ids")), PnpNames::default());
    }
}
