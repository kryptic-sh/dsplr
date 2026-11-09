//! Generates the Wayland protocol code the `wlr` backend speaks, with
//! `crcbl-wl-scanner`, against `crcbl_wayland::ffi`.
//!
//! The XML is vendored in `protocols/`, unmodified copies taken from crcbl,
//! whose READMEs record their upstream:
//!
//! - `wayland.xml`: `wayland`'s `protocol/wayland.xml`, 1.25.0. Generated
//!   only because the scanner resolves the `wl_output` that
//!   wlr-output-management names within one emit set.
//! - `wlr-output-management-unstable-v1.xml`: `wlr-protocols`,
//!   `unstable/wlr-output-management-unstable-v1.xml` (commit `2b8d4332`), v4.
//!
//! Both carry their licence (MIT, and HPND) inside the file.

use std::path::{Path, PathBuf};

use crcbl_wl_scanner::{Options, Source, parse_protocol};

const PROTOCOLS: &[(&str, &str)] = &[
    ("wayland", "protocols/wayland.xml"),
    (
        "wlr_output_management",
        "protocols/wlr-output-management-unstable-v1.xml",
    ),
];

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets this"));
    let mut sources = Vec::with_capacity(PROTOCOLS.len());
    for (module, relative) in PROTOCOLS {
        let path = manifest.join(relative);
        println!("cargo::rerun-if-changed={}", path.display());
        let xml = std::fs::read_to_string(&path)
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
