//! Generates the Wayland protocol code the `wlr` and `kde` backends speak,
//! with `crcbl-wl-scanner`, against `crcbl_wayland::ffi`.
//!
//! The XML is vendored in `protocols/` as unmodified copies:
//!
//! - `wayland.xml`: `wayland`'s `protocol/wayland.xml`, 1.25.0, taken from
//!   crcbl, whose README records its upstream. Generated because the scanner
//!   resolves the `wl_output` that wlr-output-management names within one
//!   emit set, and for `wl_registry` and `wl_callback`.
//! - `wlr-output-management-unstable-v1.xml`: `wlr-protocols`,
//!   `unstable/wlr-output-management-unstable-v1.xml` (commit `2b8d4332`), v4,
//!   taken from crcbl likewise.
//! - `kde-output-device-v2.xml` (`kde_output_device_registry_v2` and
//!   `kde_output_device_v2` v26), `kde-output-management-v2.xml` (v22) and
//!   `kde-output-order-v1.xml` (v1): KDE's `plasma-wayland-protocols`,
//!   `src/protocols/`, tag `v1.23.0` (commit `c5ac4db8`), from
//!   <https://invent.kde.org/libraries/plasma-wayland-protocols>. KWin 6.8
//!   builds against that release (its `CMakeLists.txt` asks for 1.23.0).
//!
//! `wayland.xml` and the wlr file carry their licences (MIT, and HPND) inside
//! the file. The KDE files carry their copyright lines and an SPDX
//! identifier, `MIT-CMU`, whose text is `protocols/LICENSES/MIT-CMU.txt`,
//! copied from the same tag's `LICENSES/`.
//!
//! The KDE files wrap their copyright in a CDATA section, which the scanner's
//! reader refuses, so each CDATA section is turned into the character data it
//! stands for before parsing (`build/cdata.rs`, tested with the library);
//! the files themselves stay as upstream wrote them.

#[path = "build/cdata.rs"]
mod cdata;

use std::path::{Path, PathBuf};

use cdata::without_cdata;
use crcbl_wl_scanner::{Options, Source, parse_protocol};

const PROTOCOLS: &[(&str, &str)] = &[
    ("wayland", "protocols/wayland.xml"),
    (
        "wlr_output_management",
        "protocols/wlr-output-management-unstable-v1.xml",
    ),
    ("kde_output_device", "protocols/kde-output-device-v2.xml"),
    (
        "kde_output_management",
        "protocols/kde-output-management-v2.xml",
    ),
    ("kde_output_order", "protocols/kde-output-order-v1.xml"),
];

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets this"));
    let mut sources = Vec::with_capacity(PROTOCOLS.len());
    for (module, relative) in PROTOCOLS {
        let path = manifest.join(relative);
        println!("cargo::rerun-if-changed={}", path.display());
        let xml = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let xml = without_cdata(&xml)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let protocol = parse_protocol(&xml)
            .unwrap_or_else(|error| panic!("cannot parse {}: {error}", path.display()));
        sources.push(Source { module, protocol });
    }

    let generated = crcbl_wl_scanner::emit(
        &sources,
        &Options {
            ffi_path: "crcbl_wayland::ffi",
        },
    )
    .unwrap_or_else(|error| panic!("cannot generate Wayland protocol code: {error}"));

    let out = Path::new(&std::env::var_os("OUT_DIR").expect("cargo sets this")).join("protocol.rs");
    std::fs::write(&out, generated)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", out.display()));
}
