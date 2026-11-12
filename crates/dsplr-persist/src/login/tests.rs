use std::ffi::OsString;
use std::fs;
use std::path::Path;

use dsplr_model::{Layout, Mode, Output, OutputEntry, OutputId, OutputState};

use super::{
    AUTOSTART_FILE, Login, SaveError, Wm, entry, exec_argument, own_restore, profile_name, save,
    string_value,
};
use crate::managed::{Comment, FileError, Ownership, ownership};
use crate::profile::{self, Profile};

const EXE: &str = "/usr/bin/dsplr";

fn exe() -> &'static Path {
    Path::new(EXE)
}

fn output(make: &str, model: &str, connector: &str, serial: &str) -> OutputEntry {
    OutputEntry {
        id: OutputId::new(make, model, serial, connector),
        output: Some(Output {
            connector: connector.into(),
            description: format!("{make} {model} {serial}").trim().to_owned(),
            physical_size_mm: None,
            make: make.into(),
            model: model.into(),
            serial: serial.into(),
            modes: vec![Mode::new(1920, 1080, 60_000)],
        }),
        state: OutputState {
            mode: Some(Mode::new(1920, 1080, 60_000)),
            ..OutputState::default()
        },
    }
}

fn layout(entries: Vec<OutputEntry>) -> Layout {
    Layout {
        entries,
        ..Default::default()
    }
}

/// `XDG_CONFIG_HOME` at `config`, and `desktop` as `XDG_CURRENT_DESKTOP`.
fn vars<'a>(config: &'a Path, desktop: Option<&'a str>) -> impl Fn(&str) -> Option<OsString> + 'a {
    move |name| match name {
        "XDG_CONFIG_HOME" => Some(config.into()),
        "XDG_CURRENT_DESKTOP" => desktop.map(OsString::from),
        _ => None,
    }
}

/// [`save`] with dsplr at [`EXE`].
fn save_at(
    var: impl Fn(&str) -> Option<OsString>,
    dir: &Path,
    layout: &Layout,
    approved: Option<&str>,
) -> Result<super::Saved, SaveError> {
    save(var, exe(), dir, layout, approved)
}

#[test]
fn the_profile_is_named_by_its_outputs_in_order_whatever_they_are_called() {
    let desk = layout(vec![
        output("Dell Inc.", "DELL U2720Q", "DP-1", "8KX/2#1"),
        output("BOE", "0x0A1B", "eDP-1", ""),
    ]);
    assert_eq!(
        profile_name(&desk).as_str(),
        "Dell Inc. DELL U2720Q 8KX_2_1 - eDP-1"
    );
    // Another monitor on the same ports is another profile.
    let other = layout(vec![
        output("Dell Inc.", "DELL U2720Q", "DP-1", "ZZZ"),
        output("BOE", "0x0A1B", "eDP-1", ""),
    ]);
    assert_ne!(profile_name(&desk), profile_name(&other));
}

#[test]
fn a_name_too_long_for_a_profile_is_a_hash_of_the_outputs() {
    let long = "M".repeat(150);
    let wide = layout(vec![
        output("Make", &long, "DP-1", "1"),
        output("Make", &long, "DP-2", "2"),
    ]);
    let name = profile_name(&wide);
    assert!(name.as_str().starts_with("outputs "), "{name}");
    assert_eq!(name.as_str().len(), "outputs ".len() + 12);
    let narrower = layout(vec![
        output("Make", &long, "DP-1", "1"),
        output("Make", &long, "DP-2", "3"),
    ]);
    assert_ne!(name, profile_name(&narrower));
}

#[test]
fn i3_and_bspwm_are_found_in_xdg_current_desktop() {
    let none = Path::new("/nowhere");
    assert_eq!(Wm::current(&vars(none, Some("i3"))), Some(Wm::I3));
    assert_eq!(
        Wm::current(&vars(none, Some("X-Foo:BSPWM"))),
        Some(Wm::Bspwm)
    );
    assert_eq!(Wm::current(&vars(none, Some("XFCE"))), None);
    assert_eq!(Wm::current(&vars(none, Some("i3-gaps-ish"))), None);
    assert_eq!(Wm::current(&vars(none, None)), None);
}

#[test]
fn saving_writes_the_profile_and_the_autostart_entry_dsplr_owns() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("config");
    let store = config.join("dsplr/profiles");
    let desk = layout(vec![output("BOE", "0x0A1B", "eDP-1", "")]);

    let saved = save_at(vars(&config, Some("Openbox")), &store, &desk, None).unwrap();
    let autostart = config.join("autostart").join(AUTOSTART_FILE);
    assert_eq!(
        saved.login,
        Login::Autostart {
            path: autostart.clone(),
            created: true,
            unknown_desktop: false,
        }
    );
    let text = fs::read_to_string(&autostart).unwrap();
    assert_eq!(text, entry(exe()).unwrap());
    assert_eq!(ownership(&text, Comment::Hash), Ownership::Managed);
    assert!(
        text.lines()
            .any(|line| line == "Exec=/usr/bin/dsplr restore --login --yes"),
        "{text}"
    );
    assert!(
        text.lines().any(|line| line == "TryExec=/usr/bin/dsplr"),
        "{text}"
    );
    assert_eq!(saved.taken, None);
    let kept = profile::load(&store, &saved.name).unwrap().expect("saved");
    assert_eq!(kept.layout.entries, desk.entries);
    assert_eq!(saved.profile, profile::path(&store, &saved.name));

    // Again: the same entry, now there, and no desktop named.
    let again = save_at(vars(&config, None), &store, &desk, None).unwrap();
    assert!(matches!(
        again.login,
        Login::Autostart {
            created: false,
            unknown_desktop: true,
            ..
        }
    ));
}

#[test]
fn an_autostart_entry_someone_else_wrote_is_asked_about_and_nothing_is_saved() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("config");
    let store = config.join("dsplr/profiles");
    let autostart = config.join("autostart").join(AUTOSTART_FILE);
    fs::create_dir_all(autostart.parent().unwrap()).unwrap();
    let theirs = "[Desktop Entry]\nExec=something else\n";
    fs::write(&autostart, theirs).unwrap();
    let desk = layout(vec![output("BOE", "0x0A1B", "eDP-1", "")]);

    let Err(SaveError::File(FileError::Conflict(conflict))) =
        save_at(vars(&config, None), &store, &desk, None)
    else {
        panic!("not a conflict");
    };
    assert_eq!(conflict.ownership, Ownership::Foreign);
    assert_eq!(conflict.existing, theirs);
    assert_eq!(fs::read_to_string(&autostart).unwrap(), theirs);
    assert!(
        profile::list(&store).unwrap().is_empty(),
        "a profile was saved"
    );

    save_at(vars(&config, None), &store, &desk, Some(theirs)).unwrap();
    assert_eq!(
        fs::read_to_string(&autostart).unwrap(),
        entry(exe()).unwrap()
    );
    assert_eq!(profile::list(&store).unwrap().len(), 1);
}

#[test]
fn under_i3_the_line_is_given_and_no_entry_is_written() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("config");
    let store = config.join("dsplr/profiles");
    let desk = layout(vec![output("BOE", "0x0A1B", "eDP-1", "")]);

    let saved = save_at(vars(&config, Some("i3")), &store, &desk, None).unwrap();
    assert_eq!(saved.login, Login::Line(Wm::I3));
    assert_eq!(
        Wm::I3.line(),
        "exec --no-startup-id dsplr restore --login --yes"
    );
    assert!(!config.join("autostart").exists());
    assert_eq!(profile::list(&store).unwrap().len(), 1);
}

#[test]
fn no_outputs_is_nothing_to_save() {
    let home = tempfile::tempdir().unwrap();
    let result = save_at(
        vars(home.path(), None),
        &home.path().join("profiles"),
        &layout(Vec::new()),
        None,
    );
    assert!(matches!(result, Err(SaveError::Empty)), "{result:?}");
    assert!(!home.path().join("autostart").exists());
}

#[test]
fn desktops_that_restore_their_own_layout_get_no_entry() {
    let none = Path::new("/nowhere");
    assert_eq!(own_restore(&vars(none, Some("XFCE"))), Some("XFCE"));
    assert_eq!(
        own_restore(&vars(none, Some("Budgie:GNOME"))),
        Some("Budgie")
    );
    assert_eq!(
        own_restore(&vars(none, Some("x-cinnamon"))),
        Some("X-Cinnamon")
    );
    assert_eq!(own_restore(&vars(none, Some("i3"))), None);
    assert_eq!(own_restore(&vars(none, None)), None);

    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("config");
    let store = config.join("dsplr/profiles");
    let desk = layout(vec![output("BOE", "0x0A1B", "eDP-1", "")]);
    let saved = save_at(vars(&config, Some("KDE")), &store, &desk, None).unwrap();
    assert_eq!(saved.login, Login::Desktop("KDE"));
    assert!(!config.join("autostart").exists());
    assert_eq!(profile::list(&store).unwrap().len(), 1);
}

#[test]
fn a_profile_of_that_name_the_user_saved_is_kept_and_the_hash_name_used() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("config");
    let store = config.join("dsplr/profiles");
    let desk = layout(vec![output("BOE", "0x0A1B", "eDP-1", "")]);
    let name = profile_name(&desk);
    // Saved by hand (`dsplr save --profile eDP-1`): unmarked, other outputs.
    let theirs = Profile {
        name: name.clone(),
        layout: layout(vec![output("AOC", "Q27G4", "eDP-1", "123")]),
        saved_unix_ms: 1,
    };
    profile::save(&store, &theirs).unwrap();

    let saved = save_at(vars(&config, Some("i3")), &store, &desk, None).unwrap();
    assert_eq!(saved.taken.as_ref(), Some(&name));
    assert!(
        saved.name.as_str().starts_with("outputs "),
        "{}",
        saved.name
    );
    assert_eq!(profile::load(&store, &name).unwrap(), Some(theirs));
    // The next save for these outputs finds its own under the hash name.
    let again = save_at(vars(&config, Some("i3")), &store, &desk, None).unwrap();
    assert_eq!(again.name, saved.name);
    assert_eq!(profile::list(&store).unwrap().len(), 2);
}

#[test]
fn a_kept_profile_is_replaced_for_its_own_outputs_and_not_for_others() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("config");
    let store = config.join("dsplr/profiles");
    let desk = layout(vec![output("BOE", "0x0A1B", "eDP-1", "")]);
    let first = save_at(vars(&config, Some("i3")), &store, &desk, None).unwrap();
    let marked = profile::load_marked(&store, &first.name).unwrap();
    assert!(marked.is_some_and(|(_, for_outputs)| for_outputs));
    let again = save_at(vars(&config, Some("i3")), &store, &desk, None).unwrap();
    assert_eq!((again.name, again.taken), (first.name.clone(), None));

    // A save kept for other outputs whose name these outputs also get: the
    // same connector with no serial, written under the same name by hand.
    let other = layout(vec![output("AOC", "Q27G4", "eDP-1", "9")]);
    let kept_for_other = Profile {
        name: first.name.clone(),
        layout: other,
        saved_unix_ms: 1,
    };
    profile::save_for_outputs(&store, &kept_for_other).unwrap();
    let saved = save_at(vars(&config, Some("i3")), &store, &desk, None).unwrap();
    assert_eq!(saved.taken.as_ref(), Some(&first.name));
    assert_ne!(saved.name, first.name);
    assert_eq!(
        profile::load(&store, &first.name).unwrap(),
        Some(kept_for_other)
    );
}

#[cfg(unix)]
#[test]
fn a_profile_that_cannot_be_written_leaves_no_entry() {
    use std::os::unix::fs::PermissionsExt;

    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("config");
    // A store that cannot be written into.
    let store = home.path().join("store");
    fs::create_dir(&store).unwrap();
    fs::set_permissions(&store, fs::Permissions::from_mode(0o555)).unwrap();
    let desk = layout(vec![output("BOE", "0x0A1B", "eDP-1", "")]);
    let result = save_at(vars(&config, None), &store, &desk, None);
    assert!(matches!(result, Err(SaveError::Profile(_))), "{result:?}");
    assert!(!config.join("autostart").join(AUTOSTART_FILE).exists());
}

#[test]
fn exec_arguments_are_quoted_as_the_desktop_entry_spec_says() {
    assert_eq!(
        exec_argument("/home/u/.cargo/bin/dsplr"),
        "/home/u/.cargo/bin/dsplr"
    );
    assert_eq!(
        exec_argument("/opt/my apps/dsplr"),
        "\"/opt/my apps/dsplr\""
    );
    assert_eq!(exec_argument("/opt/100%/dsplr"), "/opt/100%%/dsplr");
    // The spec's own case: a literal backslash in a quoted argument is four
    // in the file, the quoting's escape and then the string's.
    assert_eq!(
        string_value(&exec_argument("/a/$b\\c")),
        "\"/a/\\\\$b\\\\\\\\c\""
    );
    let text = entry(Path::new("/opt/my apps/dsplr")).unwrap();
    assert!(
        text.contains("\nExec=\"/opt/my apps/dsplr\" restore --login --yes\n"),
        "{text}"
    );
    assert!(text.contains("\nTryExec=/opt/my apps/dsplr\n"), "{text}");
    assert!(matches!(
        entry(Path::new("/opt/a\nb")),
        Err(SaveError::Exe(_))
    ));
}
