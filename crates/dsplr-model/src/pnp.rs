//! A monitor's make as the Wayland compositors dsplr drives name it, from
//! the EDID's PNP vendor code (`DEL`): the company name hwdata's `pnp.ids`
//! gives it, as libdisplay-info looks it up for wlroots and Hyprland. The
//! EDID, X11's RandR and Mutter give the code, so it is turned into that
//! name before a monitor's identity is made, and a profile saved on one
//! desktop names the monitor as another does.
//!
//! Reading the file is the caller's (`dsplr_persist::pnp`); this module
//! only reads its text.

use std::collections::HashMap;

/// The company names of `pnp.ids`, by PNP code, or no table at all when
/// the file could not be read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PnpNames {
    names: Option<HashMap<String, String>>,
}

impl PnpNames {
    /// The names in `text`, a `pnp.ids`, read as libdisplay-info 0.3.0's
    /// `tool/gen-search-table.py` reads it: each line split once at the
    /// first run of whitespace into a code and a name, the name trimmed, a
    /// line whose code is not three characters (or with no name) skipped,
    /// and a later line for a code replacing an earlier one.
    pub fn parse(text: &str) -> Self {
        let names = text
            .lines()
            .filter_map(|line| {
                let line = line.trim_start();
                let (code, name) = line.split_once(char::is_whitespace)?;
                let name = name.trim();
                (code.chars().count() == 3 && !name.is_empty())
                    .then(|| (code.to_owned(), name.to_owned()))
            })
            .collect();
        Self { names: Some(names) }
    }

    /// The make for a vendor a backend reported: a PNP code (three letters
    /// `A` to `Z`) becomes what libdisplay-info 0.3.0's `di_info_get_make`
    /// gives for it: the name, with each byte outside printable ASCII
    /// written `\xNN`, or `PNP(XYZ)` for a code the table lacks. With no
    /// table the code is kept, and anything that is not a code (a name a
    /// backend already resolved, Mutter's `MetaVendor`) is kept as given.
    pub fn make(&self, vendor: &str) -> String {
        let is_code = vendor.len() == 3 && vendor.bytes().all(|byte| byte.is_ascii_uppercase());
        match &self.names {
            Some(names) if is_code => names
                .get(vendor)
                .map_or_else(|| format!("PNP({vendor})"), |name| escaped(name)),
            _ => vendor.to_owned(),
        }
    }
}

/// `text` as libdisplay-info's `encode_ascii_string` writes it: printable
/// ASCII as it is, every other byte as `\xNN`.
fn escaped(text: &str) -> String {
    text.bytes()
        .map(|byte| {
            if (0x20..0x7f).contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("\\x{byte:02x}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::PnpNames;

    /// Lines as hwdata 0.412's `pnp.ids` writes them: a code, a tab, a name.
    const PNP_IDS: &str =
        "AUS\tASUSTek COMPUTER INC\nDEL\tDell Inc.\nDLL\tDell Inc\nLNX\tThe Linux Foundation\n";

    #[test]
    fn a_code_becomes_its_company_name() {
        let names = PnpNames::parse(PNP_IDS);
        assert_eq!(names.make("DEL"), "Dell Inc.");
        assert_eq!(names.make("DLL"), "Dell Inc");
        assert_eq!(names.make("AUS"), "ASUSTek COMPUTER INC");
        assert_eq!(names.make("LNX"), "The Linux Foundation");
    }

    #[test]
    fn a_code_not_in_the_table_is_named_as_libdisplay_info_names_it() {
        assert_eq!(PnpNames::parse(PNP_IDS).make("ZZZ"), "PNP(ZZZ)");
    }

    #[test]
    fn without_a_table_or_a_code_the_vendor_is_kept() {
        assert_eq!(PnpNames::default().make("DEL"), "DEL");
        let names = PnpNames::parse(PNP_IDS);
        for vendor in ["MetaVendor", "Dell Inc.", "del", "DE", "DELL", ""] {
            assert_eq!(names.make(vendor), vendor);
        }
    }

    #[test]
    fn lines_are_read_as_libdisplay_info_reads_them() {
        let names = PnpNames::parse(
            "ABC   Spaced  Name  \r\nTOOLONG\tSkipped\nAB\tSkipped\nEMP\nABC\tLater Wins\n\nDEF\tNot\x01Printable \u{e9}\n",
        );
        assert_eq!(names.make("ABC"), "Later Wins");
        assert_eq!(names.make("EMP"), "PNP(EMP)", "a line with no name");
        assert_eq!(names.make("DEF"), "Not\\x01Printable \\xc3\\xa9");
        let spaced = PnpNames::parse("ABC   Spaced  Name  \r\n");
        assert_eq!(spaced.make("ABC"), "Spaced  Name");
    }
}
