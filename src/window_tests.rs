//! Tests for the window: the command line, the filesystem and the clipboard.

use super::*;
use crate::clip;
use crate::document::{App, Mode};
use crate::testutil::{argv, tmp_md};
use std::path::PathBuf;

#[test]
fn a_bare_path_defaults_to_view_mode() {
    let Parsed::Run(a) = parse_args(argv("notes.md")) else {
        panic!("expected a run")
    };
    assert_eq!(a.mode, None, "no flag means the caller picks the default");
    assert_eq!(a.path, Some(PathBuf::from("notes.md")));
}

#[test]
fn edit_and_view_flags_set_the_mode() {
    let Parsed::Run(a) = parse_args(argv("-e notes.md")) else {
        panic!("expected a run")
    };
    assert_eq!(a.mode, Some(Mode::Edit));
    let Parsed::Run(b) = parse_args(argv("--view notes.md")) else {
        panic!("expected a run")
    };
    assert_eq!(b.mode, Some(Mode::View));
}

#[test]
fn font_flags_consume_their_value() {
    let Parsed::Run(a) = parse_args(argv("--font Fira --mono JBMono notes.md")) else {
        panic!("expected a run")
    };
    assert_eq!(a.body_family.as_deref(), Some("Fira"));
    assert_eq!(a.code_family.as_deref(), Some("JBMono"));
    assert_eq!(a.path, Some(PathBuf::from("notes.md")));
}

#[test]
fn a_font_flag_without_a_value_is_an_error() {
    assert!(matches!(parse_args(argv("--font")), Parsed::Err(_)));
    assert!(matches!(parse_args(argv("--mono")), Parsed::Err(_)));
    // A value that looks like a flag is still consumed as the family,
    // so the error is about the missing file, not a dangling flag.
    assert!(matches!(parse_args(argv("--font -e x.md")), Parsed::Run(_)));
}

#[test]
fn an_unknown_flag_is_an_error_not_a_filename() {
    let Parsed::Err(e) = parse_args(argv("--nope notes.md")) else {
        panic!("expected an error")
    };
    assert!(e.contains("--nope"), "message should name the option: {e}");
}

#[test]
fn two_files_are_rejected() {
    assert!(matches!(parse_args(argv("a.md b.md")), Parsed::Err(_)));
}

#[test]
fn help_short_circuits() {
    assert!(matches!(parse_args(argv("--help")), Parsed::Help));
    assert!(matches!(parse_args(argv("-h")), Parsed::Help));
}

/// Offscreen proof that system-font glyphs actually rasterize into the
/// framebuffer, including Unicode via fallback.

#[test]
fn clipboard_degrades_gracefully() {
    let copied = clip::copy("mimd clipboard test");
    let pasted = clip::paste();
    if copied {
        assert!(
            pasted.is_some(),
            "copy claimed success but paste found nothing"
        );
        assert_eq!(pasted.as_deref(), Some("mimd clipboard test"));
    } else {
        assert!(pasted.is_none(), "paste worked but copy reported failure");
    }
}

#[test]
fn clipboard_rejects_empty_input() {
    assert!(!clip::copy(""));
}

#[test]
fn open_rejects_non_markdown_paths() {
    let bad = tmp_md("bad.txt", "hi");
    let r = open_path(bad.to_str().unwrap());
    assert!(r.is_err(), "a .txt file must be refused");
    assert!(open_path("/etc/passwd").is_err());
    assert!(open_path("/etc/shadow").is_err());
    assert!(open_path("").is_err());
    assert!(open_path("   ").is_err());
    let _ = std::fs::remove_file(bad);
}

#[test]
fn open_reports_a_missing_file_without_panicking() {
    assert!(open_path("/nonexistent/definitely-not-here.md").is_err());
}

#[test]
fn open_accepts_md_and_markdown() {
    let a = tmp_md("a.md", "# A\n");
    assert!(open_path(a.to_str().unwrap()).is_ok());
    let b = tmp_md("b.markdown", "# B\n");
    assert!(open_path(b.to_str().unwrap()).is_ok());
    let _ = std::fs::remove_file(a);
    let _ = std::fs::remove_file(b);
}

#[test]
fn open_expands_tilde() {
    // A ~ path must not be treated as a literal directory named "~".
    std::env::set_var("HOME", "/tmp");
    let r = open_path("~/mimd-should-not-exist-xyz.md");
    assert!(r.is_err());
    assert!(
        !r.unwrap_err().contains("~"),
        "the tilde should have been expanded before reporting"
    );
}

#[test]
fn save_then_open_round_trips() {
    let p = tmp_md("round.md", "old\n");
    let mut a = App::open("old\n".into(), Some(p.clone()), Mode::Edit);
    a.insert("# Fresh\n\n- item\n");
    a.save();
    assert!(!a.dirty, "save must clear the dirty flag");
    assert!(a.status.starts_with("saved"));
    let (back, text) = open_path(p.to_str().unwrap()).expect("reopen");
    assert_eq!(back, p);
    assert_eq!(text, a.text());
    assert!(text.contains("Fresh"));
    let _ = std::fs::remove_file(p);
}

#[test]
fn saving_without_a_path_is_refused_not_silent() {
    let mut a = App::open("unsaved work\n".into(), None, Mode::Edit);
    a.save();
    assert!(a.status.contains("no file path"), "got: {}", a.status);
}

#[test]
fn save_refuses_non_markdown_targets() {
    let bad = std::env::temp_dir().join("mimd-test-save.txt");
    let _ = std::fs::write(&bad, "x");
    let mut a = App::open("data\n".into(), Some(bad.clone()), Mode::Edit);
    a.insert("edited\n");
    assert!(a.dirty);
    a.save();
    assert!(a.status.contains("non-Markdown"), "got: {}", a.status);
    assert!(a.dirty, "a refused save must leave the buffer dirty");
    let _ = std::fs::remove_file(bad);
}

/// Drive `mouse_step` the way the event loop does, with no display.
#[test]
fn a_trailing_newline_survives_open_and_save() {
    // Files conventionally end in a newline; dropping it would rewrite
    // every file the user merely opened.
    let a = App::open("# T\n\nbody\n".into(), None, Mode::View);
    assert!(a.text().ends_with('\n'));
    assert_eq!(a.text(), "# T\n\nbody\n");
}

#[test]
fn a_file_without_a_trailing_newline_keeps_its_shape() {
    let a = App::open("# T\n\nbody".into(), None, Mode::View);
    assert_eq!(a.text(), "# T\n\nbody");
    assert!(!a.text().ends_with('\n'));
}

#[test]
fn round_trip_through_disk_preserves_bytes() {
    for original in ["# T\n\nbody\n", "# T\n\nbody", "one\n", "one", ""] {
        let p = tmp_md("bytes.md", original);
        let (path, text) = open_path(p.to_str().unwrap()).expect("reopen");
        let mut a = App::open(text, Some(path.clone()), Mode::Edit);
        a.save();
        let after = std::fs::read_to_string(&p).unwrap();
        assert_eq!(after, original, "round-trip changed {original:?}");
        let _ = std::fs::remove_file(p);
    }
}
