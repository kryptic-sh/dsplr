//! A monitor's EDID base block (VESA E-EDID, structure version 1, any
//! revision, as libdisplay-info reads it), as RandR's `EDID` output property
//! and autorandr's profiles carry it: who made the monitor, which one it is,
//! and its preferred mode.
//!
//! Only the 128-byte base block is read; extension blocks after it (CTA-861,
//! DisplayID) are accepted and left alone.

use crate::{OutputId, PnpNames};

/// The length of the base block, and of each extension block.
const BLOCK: usize = 128;

/// The fixed pattern every EDID starts with.
const HEADER: [u8; 8] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];

/// Where the four 18-byte descriptors start in the base block.
const DESCRIPTORS: [usize; 4] = [54, 72, 90, 108];

/// Display descriptor tags (the fourth byte of a descriptor whose pixel
/// clock is zero).
const TAG_SERIAL: u8 = 0xff;
const TAG_TEXT: u8 = 0xfe;
const TAG_NAME: u8 = 0xfc;

/// Why bytes are not an EDID dsplr can read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EdidError {
    #[error("{0} bytes is shorter than an EDID's 128-byte base block")]
    TooShort(usize),
    #[error("the EDID does not start with its fixed header")]
    BadHeader,
    #[error("the EDID's base block checksum is wrong (its bytes sum to {0:#04x}, not 0)")]
    BadChecksum(u8),
    #[error("EDID structure version {version}.{revision} is not a 1.x one")]
    UnsupportedVersion { version: u8, revision: u8 },
}

/// When the monitor was made, as the EDID says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Made {
    /// The year of manufacture, and the week when the EDID says it (weeks
    /// 1 to 54; 0 is "not given").
    Week { week: Option<u8>, year: u16 },
    /// A model year instead (week `0xff`, read so in every 1.x revision, as
    /// `di-edid-decode` reads it).
    ModelYear(u16),
}

/// The preferred mode: the first detailed timing descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// The pixel clock, in kilohertz (the EDID gives it in 10 kHz steps).
    pub pixel_clock_khz: u32,
    pub width: u16,
    pub height: u16,
    /// The refresh rate the totals give, in millihertz, to the nearest one:
    /// the pixel clock over the total pixels per frame (per field when
    /// [`Timing::interlaced`]); 0 when the totals are 0.
    pub refresh_mhz: u32,
    pub interlaced: bool,
    /// The image size in millimetres; `None` when the EDID gives none.
    pub image_size_mm: Option<(u16, u16)>,
}

/// A parsed EDID base block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edid {
    /// The three-letter PNP vendor code; `None` when a letter of it is out
    /// of `A` to `Z`, which leaves the rest of the EDID readable.
    pub vendor: Option<String>,
    /// The manufacturer's product code.
    pub product: u16,
    /// The serial number; `None` when it is 0 (not given).
    pub serial_number: Option<u32>,
    pub made: Made,
    /// The structure version and revision: 1, and any revision.
    pub version: (u8, u8),
    /// The maximum image size in centimetres; `None` when either is 0 (not
    /// given, or in EDID 1.4 an aspect ratio instead).
    pub size_cm: Option<(u8, u8)>,
    /// The display product name descriptor (`0xfc`).
    pub name: Option<String>,
    /// The display product serial number descriptor (`0xff`).
    pub serial: Option<String>,
    /// Each alphanumeric data string descriptor (`0xfe`), in order.
    pub texts: Vec<String>,
    /// The first detailed timing, which EDID 1.3 and later make the
    /// preferred mode (earlier revisions only when their feature byte says
    /// so, which is not read); `None` when no descriptor is one.
    pub preferred: Option<Timing>,
}

/// The make, model and serial of a monitor, as the Wayland compositors
/// dsplr drives name it from the same EDID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// The make: the PNP vendor code as a company name, as wlroots and
    /// Hyprland report it ([`PnpNames::make`]).
    pub make: Option<String>,
    pub model: String,
    pub serial: Option<String>,
}

impl Edid {
    /// Reads `bytes`: the base block, then any extension blocks, which are
    /// left alone.
    ///
    /// # Errors
    ///
    /// When there is no whole base block, its header or checksum is wrong,
    /// or its structure version is not 1 (libdisplay-info 0.3.0 reads any
    /// 1.x, and refuses 0.x and 2.x).
    pub fn parse(bytes: &[u8]) -> Result<Self, EdidError> {
        let base = bytes.get(..BLOCK).ok_or(EdidError::TooShort(bytes.len()))?;
        if base[..8] != HEADER {
            return Err(EdidError::BadHeader);
        }
        let sum = base.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        if sum != 0 {
            return Err(EdidError::BadChecksum(sum));
        }
        let version = (base[18], base[19]);
        if version.0 != 1 {
            return Err(EdidError::UnsupportedVersion {
                version: version.0,
                revision: version.1,
            });
        }
        let year = 1990 + u16::from(base[17]);
        let made = match base[16] {
            0xff => Made::ModelYear(year),
            0 => Made::Week { week: None, year },
            week => Made::Week {
                week: Some(week),
                year,
            },
        };
        let size_cm = (base[21] != 0 && base[22] != 0).then_some((base[21], base[22]));
        let mut edid = Self {
            vendor: vendor(u16::from_be_bytes([base[8], base[9]])),
            product: u16::from_le_bytes([base[10], base[11]]),
            serial_number: Some(u32::from_le_bytes([base[12], base[13], base[14], base[15]]))
                .filter(|&serial| serial != 0),
            made,
            version,
            size_cm,
            name: None,
            serial: None,
            texts: Vec::new(),
            preferred: None,
        };
        for at in DESCRIPTORS {
            let descriptor = &base[at..at + 18];
            if descriptor[..2] != [0, 0] {
                edid.preferred = edid.preferred.or_else(|| Some(timing(descriptor)));
                continue;
            }
            match descriptor[3] {
                TAG_NAME => edid.name = edid.name.or_else(|| Some(text(descriptor))),
                TAG_SERIAL => edid.serial = edid.serial.or_else(|| Some(text(descriptor))),
                TAG_TEXT => edid.texts.push(text(descriptor)),
                _ => {}
            }
        }
        Ok(edid)
    }

    /// The make, model and serial libdisplay-info gives (0.3.0, run here
    /// against EDIDs built for it), which wlroots 0.20 and Hyprland's
    /// aquamarine 0.16 link: the PNP code's company name from `names`
    /// ([`PnpNames::make`]); the product name descriptor, else the product
    /// code as `0x%04X`; the serial number descriptor, else the serial
    /// number as `0x%08X`, else none.
    pub fn identity(&self, names: &PnpNames) -> Identity {
        Identity {
            make: self.vendor.as_deref().map(|code| names.make(code)),
            model: self
                .name
                .clone()
                .unwrap_or_else(|| format!("0x{:04X}", self.product)),
            serial: self
                .serial
                .clone()
                .or_else(|| self.serial_number.map(|serial| format!("0x{serial:08X}"))),
        }
    }

    /// The identity of the monitor on `connector`, by [`OutputId::new`]'s
    /// rule from [`Edid::identity`].
    pub fn output_id(&self, connector: &str, names: &PnpNames) -> OutputId {
        let identity = self.identity(names);
        OutputId::new(
            identity.make.as_deref().unwrap_or_default(),
            &identity.model,
            identity.serial.as_deref().unwrap_or_default(),
            connector,
        )
    }
}

/// The PNP code packed in `id`: three 5-bit letters, `1` for `A` to `26`
/// for `Z`, most significant first. `None` when one is out of that range.
fn vendor(id: u16) -> Option<String> {
    [10, 5, 0]
        .into_iter()
        .map(|shift| match (id >> shift) & 0x1f {
            letter @ 1..=26 => Some(char::from(b'A' + u8::try_from(letter).ok()? - 1)),
            _ => None,
        })
        .collect()
}

/// A display descriptor's 13 bytes of text, as libdisplay-info 0.3.0 gives
/// it: up to a line feed or a NUL, which end it (spaces before a line feed
/// are kept, and trailing spaces where there is none); each byte from 0x20
/// to 0x7e as it is, a backslash included, and any other as `\xNN` in
/// lowercase hex.
fn text(descriptor: &[u8]) -> String {
    descriptor[5..]
        .iter()
        .take_while(|&&byte| byte != b'\n' && byte != 0)
        .map(|&byte| match byte {
            0x20..=0x7e => char::from(byte).to_string(),
            _ => format!("\\x{byte:02x}"),
        })
        .collect()
}

/// A detailed timing descriptor.
fn timing(d: &[u8]) -> Timing {
    let pixel_clock_khz = u32::from(u16::from_le_bytes([d[0], d[1]])) * 10;
    let high = |low: u8, nibble: u8| u16::from(low) | (u16::from(nibble) << 8);
    let width = high(d[2], d[4] >> 4);
    let h_blank = high(d[3], d[4] & 0x0f);
    let height = high(d[5], d[7] >> 4);
    let v_blank = high(d[6], d[7] & 0x0f);
    let image = (high(d[12], d[14] >> 4), high(d[13], d[14] & 0x0f));
    let total = u64::from(width + h_blank) * u64::from(height + v_blank);
    // Hz times 1000: kHz times 10^6, rounded to the nearest; 0 when the
    // totals are, and the most a u32 holds past that.
    let refresh_mhz = (u64::from(pixel_clock_khz) * 1_000_000 + total / 2)
        .checked_div(total)
        .map_or(0, |mhz| u32::try_from(mhz).unwrap_or(u32::MAX));
    Timing {
        pixel_clock_khz,
        width,
        height,
        refresh_mhz,
        interlaced: d[17] & 0x80 != 0,
        image_size_mm: (image.0 != 0 && image.1 != 0).then_some(image),
    }
}

#[cfg(test)]
mod tests {
    use super::{Edid, EdidError, Identity, Made, Timing};
    use crate::{OutputId, PnpNames};

    /// An EDID built byte by byte per the VESA E-EDID layout: vendor `DEL`,
    /// product 0x12AB, serial number 0x0000ABCD, week 12 of 2020, EDID 1.4,
    /// 60 cm x 34 cm, a 2560x1440 timing at 241.5 MHz (totals 2720 x 1481),
    /// and no text descriptors; `edit` changes it before the checksum.
    fn built(edit: impl FnOnce(&mut [u8; 128])) -> [u8; 128] {
        let mut b = [0u8; 128];
        b[..8].copy_from_slice(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]);
        // D=4, E=5, L=12: 00100 00101 01100.
        let vendor: u16 = (4 << 10) | (5 << 5) | 12;
        b[8..10].copy_from_slice(&vendor.to_be_bytes());
        b[10..12].copy_from_slice(&0x12abu16.to_le_bytes());
        b[12..16].copy_from_slice(&0x0000_abcdu32.to_le_bytes());
        b[16] = 12;
        b[17] = 30;
        b[18] = 1;
        b[19] = 4;
        b[21] = 60;
        b[22] = 34;
        // The detailed timing: 24150 x 10 kHz; 2560 active, 160 blanking;
        // 1440 active, 41 blanking; 597 mm x 336 mm.
        let d = &mut b[54..72];
        d[..2].copy_from_slice(&24_150u16.to_le_bytes());
        d[2] = 0x00;
        d[3] = 0xa0;
        d[4] = 0xa0;
        d[5] = 0xa0;
        d[6] = 0x29;
        d[7] = 0x50;
        d[12] = 0x55;
        d[13] = 0x50;
        d[14] = 0x21;
        // The other three: dummy descriptors (tag 0x10).
        for at in [72, 90, 108] {
            b[at + 3] = 0x10;
        }
        edit(&mut b);
        checksum(&mut b);
        b
    }

    /// Sets the last byte so the base block sums to 0.
    fn checksum(b: &mut [u8; 128]) {
        let sum = b[..127]
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        b[127] = 0u8.wrapping_sub(sum);
    }

    /// A display descriptor with `tag` and `text` at `at`.
    fn descriptor(b: &mut [u8; 128], at: usize, tag: u8, text: &[u8; 13]) {
        b[at..at + 5].copy_from_slice(&[0, 0, 0, tag, 0]);
        b[at + 5..at + 18].copy_from_slice(text);
    }

    /// A whole EDID 1.4, built byte by byte: [`built`]'s, with the VESA DMT
    /// 1024x768 at 60 Hz timing (65 MHz; totals 1344 x 806, so 60.003840
    /// Hz), 35 cm x 26 cm, and a product name, serial and text descriptor.
    fn full() -> [u8; 128] {
        built(|b| {
            // A digital input; RGB colour, and the first detailed timing the
            // preferred and native one.
            b[20] = 0x80;
            b[24] = 0x02;
            // sRGB's chromaticities in 1024ths: red (655, 338), green (307,
            // 614), blue (154, 61), white (320, 337); the low two bits of
            // each first.
            let (low, high) = chromaticities([655, 338, 307, 614, 154, 61, 320, 337]);
            b[25..27].copy_from_slice(&low);
            b[27..35].copy_from_slice(&high);
            // No standard timings: each the unused code 0x0101.
            b[38..54].fill(0x01);
            b[21] = 35;
            b[22] = 26;
            let d = &mut b[54..72];
            d.fill(0);
            d[..2].copy_from_slice(&6_500u16.to_le_bytes());
            // 1024 active, 320 blanking; 768 active, 38 blanking.
            d[2] = 0x00;
            d[3] = 0x40;
            d[4] = 0x41;
            d[5] = 0x00;
            d[6] = 0x26;
            d[7] = 0x30;
            // Sync: front porch 24, width 136; 3, 6.
            d[8] = 24;
            d[9] = 136;
            d[10] = 0x36;
            // 355 mm x 266 mm; digital separate sync, both negative.
            d[12] = 0x63;
            d[13] = 0x0a;
            d[14] = 0x11;
            d[17] = 0x18;
            descriptor(b, 72, 0xff, b"TEST0001\n    ");
            descriptor(b, 90, 0xfe, b"dsplr\n       ");
            descriptor(b, 108, 0xfc, b"Test XGA\n    ");
        })
    }

    /// Red, green, blue and white x and y, in 1024ths, as the base block
    /// packs them: the low two bits of each in two bytes, then the high
    /// eight.
    fn chromaticities(values: [u16; 8]) -> ([u8; 2], [u8; 8]) {
        let low = |four: &[u16]| {
            four.iter()
                .fold(0u8, |byte, value| (byte << 2) | (*value & 3) as u8)
        };
        let high = values.map(|value| (value >> 2) as u8);
        ([low(&values[..4]), low(&values[4..])], high)
    }

    #[test]
    fn a_whole_edid_decodes_as_edid_decode_does() {
        // Each value as `di-edid-decode` (libdisplay-info 0.3.0) decodes
        // these bytes, which it finds conformant: manufacturer DEL, model 4779 (0x12AB), serial number
        // 43981, week 12 of 2020, EDID 1.4, 35 cm x 26 cm, DTD 1 1024x768
        // at 60.003840 Hz and 65 MHz (355 mm x 266 mm), product serial
        // 'TEST0001', alphanumeric data string 'dsplr', product name 'Test
        // XGA'.
        let edid = Edid::parse(&full()).unwrap();
        assert_eq!(edid.vendor.as_deref(), Some("DEL"));
        assert_eq!(edid.product, 0x12ab);
        assert_eq!(edid.serial_number, Some(43_981));
        assert_eq!(
            edid.made,
            Made::Week {
                week: Some(12),
                year: 2020
            }
        );
        assert_eq!(edid.version, (1, 4));
        assert_eq!(edid.size_cm, Some((35, 26)));
        assert_eq!(edid.name.as_deref(), Some("Test XGA"));
        assert_eq!(edid.serial.as_deref(), Some("TEST0001"));
        assert_eq!(edid.texts, ["dsplr"]);
        assert_eq!(
            edid.preferred,
            Some(Timing {
                pixel_clock_khz: 65_000,
                width: 1024,
                height: 768,
                // 60.003840 Hz, to the nearest millihertz.
                refresh_mhz: 60_004,
                interlaced: false,
                image_size_mm: Some((355, 266)),
            })
        );
        // As libdisplay-info 0.3.0 gives it (`di_info_get_model`,
        // `di_info_get_serial`); with no names table the make is the code.
        assert_eq!(
            edid.identity(&PnpNames::default()),
            Identity {
                make: Some("DEL".into()),
                model: "Test XGA".into(),
                serial: Some("TEST0001".into()),
            }
        );
        assert_eq!(
            edid.output_id("DP-1", &PnpNames::default()),
            OutputId::Edid {
                make: "DEL".into(),
                model: "Test XGA".into(),
                serial: "TEST0001".into(),
            }
        );
        // With hwdata's names, the company name, as libdisplay-info gives it.
        let names = PnpNames::parse("DEL\tDell Inc.\n");
        assert_eq!(edid.identity(&names).make.as_deref(), Some("Dell Inc."));
        assert_eq!(
            edid.output_id("DP-1", &names),
            OutputId::Edid {
                make: "Dell Inc.".into(),
                model: "Test XGA".into(),
                serial: "TEST0001".into(),
            }
        );
    }

    #[test]
    fn every_edid_1_x_is_read_as_libdisplay_info_reads_it() {
        // libdisplay-info 0.3.0 reads this EDID as revisions 1.0 to 1.5 and
        // 1.255 alike, and refuses it as 2.0 or 0.0.
        for revision in [0, 1, 2, 3, 4, 5, 255] {
            let edid = Edid::parse(&built(|b| b[19] = revision)).unwrap();
            assert_eq!(edid.version, (1, revision));
            assert_eq!(
                edid.identity(&PnpNames::default()).model,
                "0x12AB",
                "1.{revision}"
            );
        }
        for version in [0, 2] {
            assert_eq!(
                Edid::parse(&built(|b| {
                    b[18] = version;
                    b[19] = 0;
                })),
                Err(EdidError::UnsupportedVersion {
                    version,
                    revision: 0
                })
            );
        }
        // Week 0xff is a model year in each: `di-edid-decode` says "Model
        // year: 2020" for 1.0, 1.1, 1.2, 1.3 and 1.4.
        for revision in [0, 2, 3, 4] {
            let edid = Edid::parse(&built(|b| {
                b[19] = revision;
                b[16] = 0xff;
            }))
            .unwrap();
            assert_eq!(edid.made, Made::ModelYear(2020), "1.{revision}");
        }
    }

    #[test]
    fn text_bytes_out_of_printable_ascii_are_written_as_libdisplay_info_writes_them() {
        // Each as libdisplay-info 0.3.0 gives the product name: a byte
        // outside 0x20 to 0x7e as `\xNN`, lowercase, a backslash as it is,
        // and a NUL ending the text as a line feed does.
        for (bytes, name) in [
            (b"Q2\x017G4  \n    ", "Q2\\x017G4  "),
            (b"Caf\xe9 Mon\n    ", "Caf\\xe9 Mon"),
            (b"A\\B\x7fC\n       ", "A\\B\\x7fC"),
            (b"A\tB\0C\rD\n     ", "A\\x09B"),
        ] {
            let edid = Edid::parse(&built(|b| descriptor(b, 108, 0xfc, bytes))).unwrap();
            assert_eq!(edid.name.as_deref(), Some(name), "{bytes:?}");
        }
    }

    #[test]
    fn a_built_edid_reads_back_field_by_field() {
        // As `di-edid-decode` (libdisplay-info 0.3.0) decodes it too: DEL,
        // model 4779, serial number 43981, week 12 of 2020, EDID 1.4, 60 cm
        // x 34 cm, DTD 1 2560x1440 at 59.950550 Hz and 241.5 MHz (597 mm x
        // 336 mm).
        let edid = Edid::parse(&built(|_| {})).unwrap();
        assert_eq!(edid.vendor.as_deref(), Some("DEL"));
        assert_eq!(edid.product, 0x12ab);
        assert_eq!(edid.serial_number, Some(0xabcd));
        assert_eq!(
            edid.made,
            Made::Week {
                week: Some(12),
                year: 2020
            }
        );
        assert_eq!(edid.version, (1, 4));
        assert_eq!(edid.size_cm, Some((60, 34)));
        assert_eq!((edid.name.as_ref(), edid.serial.as_ref()), (None, None));
        assert_eq!(
            edid.preferred,
            Some(Timing {
                pixel_clock_khz: 241_500,
                width: 2560,
                height: 1440,
                // 241.5 MHz over 2720 x 1481: 59.951 Hz.
                refresh_mhz: 59_951,
                interlaced: false,
                image_size_mm: Some((597, 336)),
            })
        );
    }

    #[test]
    fn without_descriptors_the_codes_name_it_as_libdisplay_info_does() {
        // libdisplay-info 0.3.0 gave model 0x12AB and serial 0x0000ABCD for
        // an EDID like this, and no serial when the number is 0.
        let identity = Edid::parse(&built(|_| {}))
            .unwrap()
            .identity(&PnpNames::default());
        assert_eq!(identity.model, "0x12AB");
        assert_eq!(identity.serial.as_deref(), Some("0x0000ABCD"));
        let none = Edid::parse(&built(|b| b[12..16].fill(0))).unwrap();
        assert_eq!(none.serial_number, None);
        assert_eq!(none.identity(&PnpNames::default()).serial, None);
        assert_eq!(
            none.output_id("DP-1", &PnpNames::default()),
            OutputId::Connector("DP-1".into()),
            "no serial: the connector"
        );
    }

    #[test]
    fn descriptors_give_the_name_serial_and_texts() {
        let edid = Edid::parse(&built(|b| {
            descriptor(b, 72, 0xfc, b"Q27G4\n       ");
            descriptor(b, 90, 0xff, b"SER1\n        ");
            // No line feed: all thirteen bytes, trailing spaces kept, as
            // libdisplay-info keeps them.
            descriptor(b, 108, 0xfe, b"ABCDEFGHIJ   ");
        }))
        .unwrap();
        assert_eq!(edid.name.as_deref(), Some("Q27G4"));
        assert_eq!(edid.serial.as_deref(), Some("SER1"));
        assert_eq!(edid.texts, ["ABCDEFGHIJ   "]);
        // The descriptors win over the codes, the serial over its number.
        assert_eq!(
            edid.identity(&PnpNames::default()),
            Identity {
                make: Some("DEL".into()),
                model: "Q27G4".into(),
                serial: Some("SER1".into()),
            }
        );
    }

    #[test]
    fn a_model_year_an_unknown_week_and_no_size_are_said() {
        let model_year = Edid::parse(&built(|b| b[16] = 0xff)).unwrap();
        assert_eq!(model_year.made, Made::ModelYear(2020));
        let no_week = Edid::parse(&built(|b| b[16] = 0)).unwrap();
        assert_eq!(
            no_week.made,
            Made::Week {
                week: None,
                year: 2020
            }
        );
        let no_size = Edid::parse(&built(|b| b[22] = 0)).unwrap();
        assert_eq!(no_size.size_cm, None);
    }

    #[test]
    fn a_vendor_letter_out_of_range_leaves_the_vendor_unknown() {
        // A zero letter: 00000 00101 01100.
        let edid = Edid::parse(&built(|b| {
            b[8..10].copy_from_slice(&((5u16 << 5) | 12).to_be_bytes());
        }))
        .unwrap();
        assert_eq!(edid.vendor, None);
        assert_eq!(edid.product, 0x12ab, "the rest is read");
        // 27 is past Z.
        let past = Edid::parse(&built(|b| {
            b[8..10].copy_from_slice(&((27u16 << 10) | (5 << 5) | 12).to_be_bytes());
        }))
        .unwrap();
        assert_eq!(past.vendor, None);
    }

    #[test]
    fn extension_blocks_are_accepted_and_left_alone() {
        let mut bytes = built(|b| b[126] = 1).to_vec();
        // A CTA-861 block whose own checksum is not checked.
        bytes.extend([0x02; 128]);
        let edid = Edid::parse(&bytes).unwrap();
        assert_eq!(edid.product, 0x12ab);
    }

    #[test]
    fn what_is_not_an_edid_says_why() {
        assert_eq!(Edid::parse(&full()[..127]), Err(EdidError::TooShort(127)));
        let mut header = full();
        header[0] = 0x01;
        assert_eq!(Edid::parse(&header), Err(EdidError::BadHeader));
        // One byte one more: the block sums to 1.
        let mut changed = full();
        changed[20] = changed[20].wrapping_add(1);
        assert_eq!(Edid::parse(&changed), Err(EdidError::BadChecksum(0x01)));
        let two = built(|b| b[18] = 2);
        assert_eq!(
            Edid::parse(&two),
            Err(EdidError::UnsupportedVersion {
                version: 2,
                revision: 4
            })
        );
    }
}
