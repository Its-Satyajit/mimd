# mimd

A tiny native Markdown viewer and editor for Linux.

```
mimd README.md
mimd -e notes.md              # open in edit mode
mimd -v notes.md              # open in view mode
mimd --font "Noto Serif" x.md # override the body font
mimd --mono "DejaVu Sans Mono" x.md
```

## Fonts

**No font is bundled.** Text is drawn with the fonts your system provides,
resolved through fontconfig and rasterized with FreeType — both bound at
runtime via `dlopen`, so neither is a build-time link dependency and no font
data lives in the binary.

| Role | Source | Used for |
|---|---|---|
| `Body` | system UI / sans, regular | paragraphs, lists, quotes, UI, status bar |
| `BodyBold` | same family, bold | headings, `**bold**`, the mode flag |
| `BodyItalic` | same family, italic | `*italic*`, the `unsaved` marker |
| `Code` | system monospace, regular | inline code and code blocks |

Emphasis is a **weight**, not a colour: `**bold**` and `*italic*` each resolve
to a real face, so they stay legible in monochrome and to colour-blind
readers. A system with no bold or italic cut for a family falls back to that
family's regular face rather than failing to render.

Because resolution goes through fontconfig, user font configuration is
respected, and missing glyphs fall back to whatever fonts are installed:

```
বাংলা   日本語   Русский   العربية   Ελληνικά
```

renders without the application knowing anything about those scripts. The
Markdown parser emits semantic roles only; no family name ever reaches it.

## Keys

| Key | Action |
|---|---|
| Ctrl+E | toggle view / edit |
| Ctrl+O | open a file (type a path, `~` works) |
| Ctrl+Z / Ctrl+Y | undo / redo (typing collapses into one step) |
| Ctrl+A / C / X / V | select all / copy / cut / paste |
| Ctrl+Q | quit (saves if modified) |
| arrows, Home/End, PgUp/PgDn | move |
| Shift+arrows | extend selection |
| Enter, Backspace, Delete, Tab | edit |
| up/down, PgUp/PgDn, mouse wheel | scroll (view mode) |
| left click / drag | place caret, extend selection (edit mode) |

Copy and paste use the system clipboard via `wl-copy`/`wl-paste` (Wayland) or
`xclip` (X11), and fall back to an in-app buffer when neither is installed —
the status bar says `(internal)` when that happens.

## Markdown subset

Headings 1-6 (sized by level), paragraphs, `-`/`*`/`+` bullets and
`1.` ordered lists (nested by indentation), `>` blockquotes, `---` rules,
and fenced code blocks. Inline: `**bold**`, `*italic*`, `` `code` ``,
`[link](url)`, backslash escapes.

Text wraps to the window width, breaking on spaces and mid-word only when a
single word is wider than the line.

Out of scope on purpose: tables, images, footnotes, raw HTML, task lists,
LaTeX, syntax highlighting, CommonMark/GFM compliance.

## Build

```bash
cargo build --release     # -> target/release/mimd
cargo test
size target/release/mimd
```

To inspect the rendered design without a display, the test suite can write a
frame to a PNG:

```bash
MIMD_SNAPSHOT=/tmp/mimd.png cargo test renders_the_window
```

Requires `libfontconfig.so.1` and `libfreetype.so.6` at runtime (present on
any normal Linux desktop). Release profile is `opt-level = "z"`, fat LTO, one
codegen unit, stripped.

## Design

mimd is a reader for prose, so it is set like a page rather than a terminal.

| Token | Value | Meaning |
|---|---|---|
| paper | `#e9eaec` | the ground: a cool light grey, neutral beside black ink |
| panel | `#dcdde2` | status bar, one step down the page |
| rule | `#b4b9bf` | hairlines |
| ink | `#1a1c1e` | body text |
| ink strong | `#000000` | headings carry full ink |
| muted | `#5e6368` | rules, quote rails, status text |
| accent | `#2f5d8a` | links and the caret — the only accent |

- **One measure.** The window is sized to a 520px column (~68 characters)
  plus 28px margins, not to the display. Past ~75 characters the eye loses
  its return sweep, which is the entire point of a document reader.
- **Weight before size.** Headings ramp 23/19/16/15px and are all bold;
  H4-H6 sit at body size and differ by weight alone.
- **Structure stays quiet.** Horizontal rules and blockquote rails are drawn
  as filled hairlines, not runs of box-drawing glyphs, so they stay exactly
  one or two pixels tall whatever the typeface does with U+2500.
- **Chrome is a pager footer.** Mode and filename on the left, one contextual
  hint or a transient message on the right. A permanent list of every
  shortcut is in this file, not on screen.

## Layout

- `src/fonts.rs` — the only place that knows about concrete fonts.
  Fontconfig + FreeType FFI, resolved through `dlopen`; glyph cache;
  per-codepoint fallback via fontconfig.
- `src/main.rs` — Markdown parser (pure, font-free), editor model, renderer,
  and the event loop.
