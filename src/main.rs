//! mimd — tiny Markdown viewer/editor.
//!
//! The binary is only wiring. Each module owns one responsibility:
//!
//! - `markdown` parses text into styled runs. Pure; no fonts, no pixels.
//! - `document` is the mutable model: text, cursor, edit history.
//! - `theme` is the palette and the page geometry.
//! - `paint` is a drawing surface: blend a pixel, draw a run, fill a rect.
//! - `layout` turns the document into positioned lines.
//! - `painter` decides what to show and draws it.
//! - `nav` answers "where is the user pointing" and "which key went down".
//! - `window` owns everything outside the process: argv, files, minifb.
//! - `icon` is the window mark.
//! - `fonts` is the only module that touches fontconfig and FreeType.

mod clip;
mod document;
mod fonts;
mod icon;
mod layout;
mod markdown;
mod nav;
mod paint;
mod painter;
mod theme;

#[cfg(test)]
mod testutil;
mod window;

fn main() {
    match window::parse_args(std::env::args().skip(1)) {
        window::Parsed::Help => {
            println!("{}", window::USAGE);
            println!("  Fonts default to the system UI and monospace fonts.");
        }
        window::Parsed::Err(msg) => {
            eprintln!("mimd: {msg}");
            eprintln!("usage: {}", window::USAGE);
            std::process::exit(2);
        }
        window::Parsed::Run(args) => {
            let window::Args {
                mode,
                body_family,
                code_family,
                path,
            } = args;
            window::run(path, mode, body_family, code_family);
        }
    }
}
