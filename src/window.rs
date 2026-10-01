//! The window: command line, files, the event loop.
//!
//! Everything that touches the outside world — argv, the filesystem, minifb,
//! other processes for the clipboard. The document model and the renderer never
//! appear here except as arguments.

#[cfg(test)]
#[path = "window_tests.rs"]
mod tests;
use std::path::PathBuf;

use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};

use crate::fonts::Fonts;
use crate::paint::Canvas;

use super::document::{App, EditKind, Mode};
use super::icon::{icon_from_logo, wayland_session};
use super::layout::{build_view_layout, widest};
use super::nav::{
    caret_x, command_key, max_scroll_px, mouse_step, run_command, view_key_pressed, wheel_scroll,
};
use super::painter::render;
use super::theme::{text_column, BODY_PT, H, MARGIN, W};

/// True for paths mimd is willing to read and write.
pub fn is_markdown(p: &std::path::Path) -> bool {
    matches!(
        p.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("md") | Some("markdown")
    )
}

/// Resolve a typed path to a readable Markdown file.
pub fn open_path(typed: &str) -> Result<(PathBuf, String), String> {
    let t = typed.trim();
    if t.is_empty() {
        return Err("no path given".into());
    }
    let mut p = PathBuf::from(t);
    if t == "~" || t.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            p = PathBuf::from(home).join(t.trim_start_matches("~/"));
        }
    }
    if !is_markdown(&p) {
        return Err(format!("{} is not a Markdown file", p.display()));
    }
    match std::fs::read_to_string(&p) {
        Ok(text) => Ok((p, text)),
        Err(e) => Err(e.to_string()),
    }
}

/// minifb reports physical keys; map the printable ones back to characters.
pub fn key_char(k: Key) -> Option<char> {
    Some(match k {
        Key::Space => ' ',
        Key::Menu => return None,
        Key::Apostrophe => '\'',
        Key::Backquote => '`',
        Key::Backslash => '\\',
        Key::Comma => ',',
        Key::Equal => '=',
        Key::LeftBracket => '[',
        Key::Minus => '-',
        Key::Period => '.',
        Key::RightBracket => ']',
        Key::Semicolon => ';',
        Key::Slash => '/',
        Key::A => 'a',
        Key::B => 'b',
        Key::C => 'c',
        Key::D => 'd',
        Key::E => 'e',
        Key::F => 'f',
        Key::G => 'g',
        Key::H => 'h',
        Key::I => 'i',
        Key::J => 'j',
        Key::K => 'k',
        Key::L => 'l',
        Key::M => 'm',
        Key::N => 'n',
        Key::O => 'o',
        Key::P => 'p',
        Key::Q => 'q',
        Key::R => 'r',
        Key::S => 's',
        Key::T => 't',
        Key::U => 'u',
        Key::V => 'v',
        Key::W => 'w',
        Key::X => 'x',
        Key::Y => 'y',
        Key::Z => 'z',
        Key::Key0 => '0',
        Key::Key1 => '1',
        Key::Key2 => '2',
        Key::Key3 => '3',
        Key::Key4 => '4',
        Key::Key5 => '5',
        Key::Key6 => '6',
        Key::Key7 => '7',
        Key::Key8 => '8',
        Key::Key9 => '9',
        _ => return None,
    })
}

/// What the command line asked for.
#[derive(Default)]
pub struct Args {
    pub mode: Option<Mode>,
    pub body_family: Option<String>,
    pub code_family: Option<String>,
    pub path: Option<PathBuf>,
}

/// Outcome of parsing: show usage, run, or fail with a message.
pub enum Parsed {
    Help,
    Run(Args),
    Err(String),
}

pub const USAGE: &str = "mimd [-v|-e] [--font FAMILY] [--mono FAMILY] FILE.md";

/// Pure argument parse. A flag that needs a value but has none is an error,
/// not a silently ignored argument.
pub fn parse_args<I: Iterator<Item = String>>(argv: I) -> Parsed {
    let mut a = Args::default();
    let mut args = argv.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-e" | "--edit" => a.mode = Some(Mode::Edit),
            "-v" | "--view" => a.mode = Some(Mode::View),
            "--font" | "--mono" => {
                let Some(v) = args.next() else {
                    return Parsed::Err(format!("{arg} needs a font family"));
                };
                if arg == "--font" {
                    a.body_family = Some(v);
                } else {
                    a.code_family = Some(v);
                }
            }
            "-h" | "--help" => return Parsed::Help,
            p if p.starts_with('-') && p.len() > 1 => {
                return Parsed::Err(format!("unknown option {p}"));
            }
            p => {
                if a.path.replace(PathBuf::from(p)).is_some() {
                    return Parsed::Err("only one file may be given".into());
                }
            }
        }
    }
    Parsed::Run(a)
}

/// Open a window and run until it closes.
pub fn run(
    path: Option<PathBuf>,
    mode: Option<Mode>,
    body_family: Option<String>,
    code_family: Option<String>,
) {
    let mode = mode.unwrap_or(Mode::View);

    // Fonts come from the OS; nothing is bundled.
    let mut fonts = match Fonts::new(body_family.as_deref(), code_family.as_deref(), BODY_PT) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("mimd: {e}");
            std::process::exit(1);
        }
    };

    let text = match path.clone() {
        // Same .md guard as the Ctrl+O prompt: the CLI must not be a way
        // around the restriction.
        Some(p) => match open_path(p.to_string_lossy().as_ref()) {
            Ok((_, t)) => t,
            Err(e) => {
                eprintln!("mimd: {e}");
                std::process::exit(1);
            }
        },
        None => {
            "# mimd\n\nA tiny Markdown viewer.\n\nPress **Ctrl+E** to edit.\n\nUnicode: বাংলা 日本語 Русский العربية\n\n```\ncode\n```\n".into()
        }
    };

    let mut app = App::open(text, path, mode);
    // Resizable: a document reader is often wider than its default window,
    // and a fixed-size window is a worse experience than a little more code.
    let opts = WindowOptions {
        resize: true,
        ..WindowOptions::default()
    };
    let mut win = match Window::new("mimd", W, H, opts) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("mimd: could not open a window: {e}");
            std::process::exit(1);
        }
    };
    // minifb's set_icon is an unconditional `unimplemented!()` on the Wayland
    // backend, so calling it there aborts the process. Wayland also has no
    // runtime icon protocol at all: the icon belongs in the .desktop file.
    if !wayland_session() {
        // 32px is what a taskbar asks for; 64 keeps it crisp when a window
        // manager scales the titlebar up on a HiDPI display.
        if let Ok(icon) = icon_from_logo() {
            win.set_icon(icon);
        }
    }
    // The buffer tracks the live window size: a resized window needs a bigger
    // buffer, and handing minifb the wrong length is a hard error.
    let (mut bw, mut bh) = (W, H);
    let mut buf = vec![0u32; bw * bh];
    let body_lh = fonts.body.line_height() as usize;

    while win.is_open() && !app.quit {
        let size = win.get_size();
        if size.0 != bw || size.1 != bh {
            bw = size.0;
            bh = size.1;
            buf = vec![0u32; bw * bh];
        }
        // Edge-triggered input for this frame. `is_key_down` is a level query
        // and fires on every frame while a key is held; `get_keys_pressed`
        // reports the transition, which is what a keystroke means.
        let pressed = win.get_keys_pressed(KeyRepeat::No);
        let shift = win.is_key_down(Key::LeftShift) || win.is_key_down(Key::RightShift);
        let ctrl = win.is_key_down(Key::LeftCtrl) || win.is_key_down(Key::RightCtrl);

        // The same band the painter draws into, in window coordinates: content
        // starts at MARGIN and stops at `limit`, the top of the footer. Both the
        // page step and the scroll cap are measured against it, so they cannot
        // disagree.
        let limit = bh as i32 - (body_lh as i32 + 6);
        let viewport_h = (limit - MARGIN as i32).max(1);

        // The open prompt swallows all input until it is answered.
        if app.prompt.is_some() {
            if pressed.contains(&Key::Escape) {
                app.prompt = None;
                app.status = "open cancelled".into();
            } else if pressed.contains(&Key::Enter) {
                let typed = app.prompt.take().unwrap_or_default();
                match open_path(&typed) {
                    Ok((path, text)) => app.load(&path, text),
                    Err(e) => app.status = format!("open failed: {e}"),
                }
            } else {
                // Backspace repeats while held, which is what you want when
                // deleting a mistyped path.
                if win
                    .get_keys_pressed(KeyRepeat::Yes)
                    .contains(&Key::Backspace)
                {
                    if let Some(p) = app.prompt.as_mut() {
                        p.pop();
                    }
                }
                for k in win.get_keys_pressed(KeyRepeat::Yes) {
                    if let Some(ch) = key_char(k) {
                        if let Some(p) = app.prompt.as_mut() {
                            p.push(ch);
                        }
                    }
                }
            }
        } else if ctrl {
            if let Some(cmd) = command_key(&pressed) {
                run_command(&mut app, cmd, shift, body_lh);
            }
        } else if app.mode == Mode::Edit {
            // Edge-triggered: a held arrow moves one column, not one per frame.
            // Backspace and Delete do repeat while held, which is what you want
            // when deleting, so they come from the repeating query below.
            for k in [
                Key::Left,
                Key::Right,
                Key::Up,
                Key::Down,
                Key::Home,
                Key::End,
                Key::PageUp,
                Key::PageDown,
                Key::Tab,
                Key::Escape,
            ] {
                if pressed.contains(&k) {
                    app.key(k, shift, body_lh);
                }
            }
            for k in win.get_keys_pressed(KeyRepeat::Yes) {
                if matches!(k, Key::Backspace | Key::Delete) {
                    app.key(k, shift, body_lh);
                }
            }
            if pressed.contains(&Key::Enter) {
                app.insert("\n");
                app.follow(body_lh);
            }
            for k in win.get_keys_pressed(KeyRepeat::Yes) {
                if let Some(ch) = key_char(k) {
                    let ch = if shift { ch.to_ascii_uppercase() } else { ch };
                    app.edit_insert(&ch.to_string(), EditKind::Type);
                    app.follow(body_lh);
                }
            }
        } else {
            // Edge-triggered, not level-triggered: one press, one line.
            // A page is one viewport of lines, which is exactly what the cap
            // below is measured against.
            let page = (viewport_h as usize / body_lh.max(1)).max(1);
            match view_key_pressed(&pressed) {
                Some(Key::Down | Key::J) => app.scroll += 1,
                Some(Key::Up | Key::K) => app.scroll = app.scroll.saturating_sub(1),
                Some(Key::Home) => app.scroll = 0,
                Some(Key::End) => app.scroll = usize::MAX,
                Some(Key::PageDown) => app.scroll += page,
                Some(Key::PageUp) => app.scroll = app.scroll.saturating_sub(page),
                _ => {}
            }
        }

        app.scroll = wheel_scroll(app.scroll, win.get_scroll_wheel().map_or(0.0, |(_, y)| y));

        let (mouse_x, mouse_y, left_down) = {
            let pos = win.get_mouse_pos(MouseMode::Clamp).unwrap_or((-1.0, -1.0));
            (
                pos.0 as i32,
                pos.1 as i32,
                win.get_mouse_down(MouseButton::Left),
            )
        };

        // Build the view layout once; input hit-testing and rendering share it.
        // The column is MEASURE_CH characters wide at most and centred in the
        // window, so widening the window adds margin rather than line length,
        // and narrowing it below the measure makes the text wrap.
        let mut scratch = Canvas {
            buf: &mut Vec::new(),
            w: 0,
            h: 0,
            fonts: &mut fonts,
        };
        let row = text_column(&mut scratch, bw as i32, app.hscroll);
        let text = app.text();
        let view = build_view_layout(&mut scratch, &text, row);

        mouse_step(&mut app, mouse_x, mouse_y, left_down, row, &mut scratch);

        let mut canvas = Canvas {
            buf: &mut buf,
            w: bw as u32,
            h: bh as u32,
            fonts: &mut fonts,
        };
        // Keep the caret visible horizontally in edit mode.
        if app.mode == Mode::Edit {
            let cx = caret_x(&mut canvas, &app.lines[app.cur.l], app.cur.c, row);
            app.follow_x(cx, row);
            // Never scroll past the end of the longest line.
            let max = widest(&app, &mut canvas) + row.x - row.w;
            let cap = max.max(0) as u32;
            if app.hscroll > cap {
                app.hscroll = cap;
            }
        } else {
            // Clamp vertical scroll so the end of the document can actually
            // be reached. This has to be measured in pixels like the renderer,
            // not in rows: heading heights and block gaps make lines taller than
            // the nominal row, so a row count left the tail off-screen.
            let cap = max_scroll_px(&view.ys, limit);
            if app.scroll > cap {
                app.scroll = cap;
            }
        }
        render(&app, &mut canvas, &view);
        drop(canvas);

        if let Err(e) = win.update_with_buffer(&buf, bw, bh) {
            eprintln!("mimd: {e}");
            return;
        }
    }
}
