//! Tests for the document model: loading, editing and the undo history.

use super::*;
use crate::document::{App, Mode, Pos};
use crate::testutil::{edit_app, tmp_md};

#[test]
fn load_resets_cursor_scroll_and_dirty() {
    let mut a = App::open("old\ncontent".into(), None, Mode::Edit);
    a.cur = Pos { l: 1, c: 3 };
    a.scroll = 7;
    a.hscroll = 40;
    a.dirty = true;
    let p = tmp_md("load.md", "# New\nbody\n");
    let text = std::fs::read_to_string(&p).unwrap();
    a.load(&p, text);
    assert_eq!(a.cur, Pos::default());
    assert_eq!(a.scroll, 0);
    assert_eq!(a.hscroll, 0);
    assert!(!a.dirty);
    assert_eq!(a.text(), "# New\nbody\n", "trailing newline preserved");
    assert_eq!(a.path.as_deref(), Some(p.as_path()));
    let _ = std::fs::remove_file(p);
}

#[test]
fn load_never_yields_an_empty_document() {
    let p = tmp_md("empty.md", "");
    let mut a = App::open("x".into(), None, Mode::View);
    a.load(&p, String::new());
    assert_eq!(
        a.lines.len(),
        1,
        "an empty file must still give one editable line"
    );
    assert!(a.text().is_empty());
    let _ = std::fs::remove_file(p);
}

#[test]
fn opening_a_new_file_does_not_leak_the_old_document() {
    let p = tmp_md("fresh.md", "fresh\n");
    let mut a = App::open("stale secret content\nsecond line".into(), None, Mode::Edit);
    a.load(&p, "fresh\n".into());
    assert!(!a.text().contains("secret"));
    assert_eq!(a.selected(), "");
    let _ = std::fs::remove_file(p);
}

/// Round-trip: write via save(), then read back through open_path().

#[test]
fn undo_restores_previous_text() {
    let mut a = edit_app("start");
    a.cur = Pos { l: 0, c: 5 };
    a.edit_insert("!", EditKind::Other);
    assert_eq!(a.text(), "start!");
    assert!(a.undo());
    assert_eq!(a.text(), "start");
    assert!(a.dirty, "undoing an edit leaves the document modified");
}

#[test]
fn undo_with_nothing_to_undo_is_harmless() {
    let mut a = edit_app("start");
    assert!(!a.undo(), "nothing to undo should report false");
    assert_eq!(a.text(), "start");
    assert!(!a.redo());
    assert_eq!(a.text(), "start");
}

#[test]
fn redo_replays_an_undone_edit() {
    let mut a = edit_app("start");
    a.cur = Pos { l: 0, c: 5 };
    a.edit_insert("!", EditKind::Other);
    assert!(a.undo());
    assert_eq!(a.text(), "start");
    assert!(a.redo());
    assert_eq!(a.text(), "start!");
}

#[test]
fn a_fresh_edit_discards_the_redo_stack() {
    let mut a = edit_app("start");
    a.cur = Pos { l: 0, c: 5 };
    a.edit_insert("!", EditKind::Other);
    a.undo();
    assert!(!a.redo.is_empty());
    a.edit_insert("?", EditKind::Other);
    assert!(a.redo.is_empty(), "a new edit must invalidate redo");
    assert!(!a.redo());
    assert_eq!(a.text(), "start?");
}

#[test]
fn a_run_of_typing_undoes_as_one_step() {
    let mut a = edit_app("");
    for ch in "hello".chars() {
        a.edit_insert(&ch.to_string(), EditKind::Type);
    }
    assert_eq!(a.text(), "hello");
    assert!(a.undo());
    assert_eq!(a.text(), "", "a whole word should undo in one step");
}

#[test]
fn movement_breaks_the_typing_run() {
    let mut a = edit_app("");
    a.edit_insert("ab", EditKind::Type);
    // A caret move must end the coalescing run.
    a.move_to(Pos { l: 0, c: 0 }, false);
    a.edit_insert("X", EditKind::Type);
    assert_eq!(a.text(), "Xab");
    assert!(a.undo());
    assert_eq!(a.text(), "ab", "only the post-move character undoes");
    assert!(a.undo());
    assert_eq!(a.text(), "");
}

#[test]
fn backspace_deletes_undo_as_one_step() {
    let mut a = edit_app("abc");
    a.cur = Pos { l: 0, c: 3 };
    a.edit_backspace();
    assert_eq!(a.text(), "ab");
    assert!(a.undo());
    assert_eq!(a.text(), "abc");
}

#[test]
fn undo_restores_the_caret_and_selection_is_cleared() {
    let mut a = edit_app("hello");
    a.cur = Pos { l: 0, c: 5 };
    a.edit_insert("!", EditKind::Other);
    a.anchor = Some(Pos { l: 0, c: 2 });
    a.undo();
    assert_eq!(
        a.cur,
        Pos { l: 0, c: 5 },
        "caret must return to where it was"
    );
    assert!(a.anchor.is_none(), "selection must not survive an undo");
}

#[test]
fn undo_history_is_bounded() {
    let mut a = edit_app("");
    for _ in 0..(MAX_UNDO + 50) {
        a.edit_insert("x", EditKind::Other);
    }
    assert!(
        a.undo.len() <= MAX_UNDO,
        "history must not grow without bound"
    );
}

#[test]
fn deleting_a_selection_is_one_undo_step() {
    let mut a = edit_app("hello world");
    a.anchor = Some(Pos { l: 0, c: 0 });
    a.cur = Pos { l: 0, c: 5 };
    a.edit_delete_selection();
    assert_eq!(a.text(), " world");
    assert!(a.undo());
    assert_eq!(a.text(), "hello world");
}

#[test]
fn typing_over_a_selection_undoes_both_at_once() {
    let mut a = edit_app("hello world");
    a.anchor = Some(Pos { l: 0, c: 0 });
    a.cur = Pos { l: 0, c: 5 };
    a.edit_insert("bye", EditKind::Type);
    assert_eq!(a.text(), "bye world");
    assert!(a.undo());
    assert_eq!(a.text(), "hello world", "the replaced text must come back");
}

#[test]
fn loading_a_file_clears_the_history() {
    let p = tmp_md("hist.md", "new\n");
    let mut a = edit_app("old\n");
    a.edit_insert("!", EditKind::Other);
    a.load(&p, "new\n".into());
    assert!(
        !a.undo(),
        "history from the previous document must not apply"
    );
    assert_eq!(a.text(), "new\n");
    let _ = std::fs::remove_file(p);
}

#[test]
fn crlf_line_endings_are_normalised_on_load() {
    let a = App::open("a\r\nb\r\n".into(), None, Mode::Edit);
    assert_eq!(a.lines, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(a.text(), "a\nb\n");
}

#[test]
fn edit_delete_and_insert() {
    let mut a = App::open("hello".into(), None, Mode::Edit);
    a.anchor = Some(Pos { l: 0, c: 0 });
    a.cur = Pos { l: 0, c: 5 };
    assert_eq!(a.selected(), "hello");
    a.del_sel();
    assert_eq!(a.text(), "");
    a.insert("hi there");
    assert_eq!(a.text(), "hi there");
    a.cur = Pos { l: 0, c: 2 };
    a.del_left();
    assert_eq!(
        (a.text(), a.cur),
        ("h there".to_string(), Pos { l: 0, c: 1 })
    );
    a.del_right();
    assert_eq!(
        (a.text(), a.cur),
        ("hthere".to_string(), Pos { l: 0, c: 1 })
    );
    assert!(a.dirty);
}

#[test]
fn multiline_selection() {
    let mut a = App::open("ab\ncd\nef".into(), None, Mode::Edit);
    a.anchor = Some(Pos { l: 0, c: 1 });
    a.cur = Pos { l: 2, c: 1 };
    assert_eq!(a.selected(), "b\ncd\ne");
    a.del_sel();
    assert_eq!(a.text(), "af");
}
