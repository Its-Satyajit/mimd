# MIMD / MDVue

> **Status:** Idea / Future Project  
> **Priority:** Low  
> **Project:** Tiny native Markdown viewer and editor  
> **Language:** Rust  
> **Target:** Linux  
> **Binary target:** ≤ 2 MB

## Overview

A deliberately tiny native Markdown viewer/editor for Linux.

The primary constraint is **binary size**. The compiled executable should ideally stay **under 2 MB** while still providing a usable Markdown viewer and a minimal text editor.

The project is intentionally not trying to compete with Obsidian, Typora, VS Code, or any other software that apparently needs a small operating system to display a heading.

The core question is:

> **How much useful Markdown viewing and editing can fit into a genuinely tiny native executable?**

---

# Naming

The project has two final name candidates.

## MIMD

**Meaning:** Minimal Markdown

```text
mimd README.md
```

Characteristics:

- Extremely short.
- Strongly aligned with the minimal philosophy.
- Technical / Unix-tool feel.
- `MD` directly communicates Markdown.
- Works well as a CLI command.

## MDVue

**Meaning:** Markdown View

```text
mdvue README.md
```

Characteristics:

- Immediately communicates Markdown viewing.
- Feels more like a desktop application name.
- `MD` clearly identifies Markdown.
- `Vue` communicates the viewing/rendering aspect.
- Still compact enough for a CLI command.

## Naming Decision

Treat **MIMD** and **MDVue** as the final candidates.

Do not introduce additional names unless a concrete technical or legal issue appears.

The final selection can be made after the first working prototype, when the name can be judged against the actual application.

Until then:

```text
MIMD / MDVue
```

---

# Goals

- Native Rust application.
- Target binary size: **≤ 2 MB**.
- Very fast startup.
- Minimal memory usage.
- Open `.md` files.
- Render a small useful subset of Markdown.
- Basic Markdown editing.
- Save files.
- Simple keyboard-driven workflow.
- No web technologies.
- No JavaScript runtime.
- No database.
- No unnecessary background services.
- Keep the dependency graph extremely small.

---

# Proposed Stack

```text
Rust
  │
  ├── Minimal windowing / framebuffer layer
  │
  ├── Custom Markdown parser
  │
  ├── Custom text editor
  │
  ├── Tiny bitmap font
  │
  └── Custom renderer
```

`minifb` is the current candidate for the windowing/rendering layer because the project prioritizes minimal dependencies and binary size.

The exact GUI/windowing library may be evaluated during implementation. The chosen library must not compromise the project's core size and simplicity goals.

---

# Markdown Support

Only implement the useful minimum.

## Block Syntax

- `# Heading`
- `## Heading`
- `### Heading`
- `#### Heading`
- `##### Heading`
- `###### Heading`
- Paragraphs
- Unordered lists
- Ordered lists
- Blockquotes
- Horizontal rules

## Inline Syntax

- **Bold**
- *Italic*
- `Inline code`
- `[Links](https://example.com)`

The parser should be a small scanner or similarly compact representation rather than a large general-purpose Markdown AST.

## Explicitly Out of Scope

- Tables
- Images
- Footnotes
- Raw HTML
- Task lists
- LaTeX
- Syntax highlighting
- Emoji rendering
- Complex Markdown extensions
- Full CommonMark compatibility
- Full GFM compatibility

The project should support a deliberately useful subset, not attempt to become a specification-compliance exercise.

---

# UI

Keep the interface extremely simple.

## Initial Design

```text
┌──────────────────────────────────────────────┐
│ MIMD / MDVue                         Ctrl+S  │
├──────────────────────────────────────────────┤
│ # My Document                                │
│                                              │
│ This is **bold** text.                       │
│                                              │
│ - Item one                                   │
│ - Item two                                   │
│                                              │
└──────────────────────────────────────────────┘
```

Use two modes:

```text
Ctrl+E
   ↓
EDIT

Ctrl+E
   ↓
VIEW
```

Avoid split-view editing initially. It adds UI complexity without being necessary for the first version.

---

# Editor Features

Minimum editing functionality:

- Text insertion
- Cursor movement
- Arrow keys
- Home / End
- Backspace
- Delete
- Enter
- Selection
- Clipboard
- Ctrl+A
- Ctrl+C
- Ctrl+V
- Ctrl+S
- Open file
- Save file
- Scrolling

Nothing fancy.

---

# Binary Size Strategy

Use aggressive release optimization:

```toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

Measure the actual result rather than assuming optimization flags performed magic.

Useful tools:

```bash
cargo build --release
strip target/release/mimd
cargo bloat --release
size target/release/mimd
```

Replace the binary name in those commands with `mdvue` if that becomes the final name.

Track at least:

```text
Binary size
Startup time
Memory usage
Dependency size
```

---

# Binary Target

Primary target:

```text
MIMD / MDVue
├── Linux native executable
├── ideally ≤ 2 MB
├── fast startup
└── minimal runtime dependencies
```

A dynamically linked Linux binary is acceptable if required to hit the size target.

A completely self-contained static binary is a secondary goal because static linking can make the 2 MB constraint substantially harder.

---

# Dependency Rules

Every dependency should justify its existence.

Avoid:

- WebView
- Tauri
- Electron
- Iced
- egui
- GTK / Qt
- Tokio
- General-purpose async runtimes
- Large Markdown engines
- Syntax-highlighting engines
- Embedded browser engines
- Large font systems
- Unnecessary background services

The final dependency graph should remain extremely small.

The application should not accumulate libraries merely because they make a tiny feature convenient.

---

# Engineering Philosophy

The codebase should follow **Effect-style engineering principles** inspired by the Effect ecosystem and the broader TypeScript functional-programming approach.

This does **not** mean introducing Effect or TypeScript into the project.

The goal is to adopt the underlying engineering model:

> **Make dependencies, failures, state, and effects explicit.**

Rust's own type system and ecosystem should be used to express these concepts idiomatically.

---

# Core Engineering Principles

## 1. Explicit Effects

Code that performs I/O, filesystem access, window operations, clipboard operations, or other external effects should make those effects obvious.

Avoid hiding significant side effects inside otherwise pure-looking functions.

Prefer:

```text
pure transformation
        ↓
explicit effect boundary
        ↓
filesystem / OS / UI
```

over:

```text
function that appears to transform data
        ↓
secretly reads files
        ↓
modifies state
        ↓
writes files
```

---

## 2. Pure Core, Imperative Shell

Separate pure logic from external effects.

```text
┌─────────────────────────────┐
│        Application UI       │
│       / Effect Shell        │
├─────────────────────────────┤
│       Application Logic     │
├─────────────────────────────┤
│     Markdown Transformation │
├─────────────────────────────┤
│        Pure Functions       │
└─────────────────────────────┘
```

The Markdown parser, document model, and document transformations should be as deterministic and pure as practical.

Filesystem, clipboard, windowing, and OS integration belong near the boundaries.

---

## 3. Make Invalid States Difficult to Represent

Prefer domain types over loosely related primitives when they provide meaningful guarantees.

Instead of:

```text
filename: String
cursor: usize
line: usize
```

prefer meaningful domain structures where appropriate:

```text
Document
DocumentId
CursorPosition
Selection
MarkdownDocument
```

The type system should carry as much correctness as reasonably possible.

---

## 4. Errors Are Data

Errors should be explicit and typed.

Avoid these in normal recoverable application paths:

```text
unwrap()
expect()
panic!()
```

Prefer domain-specific errors such as:

```text
OpenDocumentError
SaveDocumentError
ParseError
RenderError
```

Errors should contain enough information to diagnose failure without forcing callers to reconstruct the cause from logs.

Panics are for violated programmer invariants, not normal user-facing failures.

---

## 5. Explicit Dependencies

Functions and components should receive the capabilities they require rather than reaching into global state.

Avoid hidden global services:

```text
GLOBAL_FILE_SYSTEM
GLOBAL_CONFIG
GLOBAL_EDITOR
GLOBAL_APP_STATE
```

Prefer explicit ownership and dependency passing.

This makes components easier to test, reason about, and replace.

---

## 6. Small Composable Functions

Prefer small functions with one clear responsibility.

Examples:

```text
read_document()
parse_markdown()
build_document()
render_document()
handle_input()
save_document()
```

Each operation should have a narrow contract.

Avoid giant functions that simultaneously:

```text
read file
parse Markdown
modify application state
render UI
save preferences
handle errors
```

Humanity has already invented spaghetti. There is no need to preserve it in Rust.

---

## 7. State Transitions Should Be Explicit

Application state should change through well-defined transitions.

Conceptually:

```text
Event
  ↓
State transition
  ↓
New state
  ↓
Effects
```

For example:

```text
OpenFile(path)
    ↓
Loading
    ↓
Loaded(document)
```

or:

```text
SaveRequested
    ↓
Saving
    ↓
Saved
```

This makes behavior easier to test without requiring a running GUI.

---

## 8. Test the Core Without the GUI

The Markdown engine, document model, editor model, and state transitions should be testable without creating a real window.

Tests should operate on:

```text
input
  ↓
pure logic
  ↓
output
```

rather than:

```text
launch application
  ↓
find window
  ↓
click something
  ↓
hope rendering worked
```

GUI tests should exist only for behavior that genuinely requires the GUI.

---

## 9. Prefer Deterministic Behavior

Given the same:

```text
document
configuration
input events
```

the core should produce the same result.

Avoid unnecessary:

- Global state
- Hidden randomness
- Background mutation
- Implicit time dependencies
- Uncontrolled concurrency

Determinism keeps the application small in both code and mental overhead.

---

## 10. Minimize Abstraction

Abstractions must earn their existence.

Before introducing a trait, framework, service container, event bus, or generic abstraction, ask:

1. Does it remove real duplication?
2. Does it make the domain clearer?
3. Does it improve testability?
4. Does it preserve the binary-size goal?
5. Is the resulting code easier to understand?

If not, do not add it.

The project is intentionally small.

---

# Effect-Style Codebase Rules

These rules should become coding conventions for the project.

## Required

- Explicit error handling.
- Explicit state transitions.
- Small composable functions.
- Pure logic wherever practical.
- Dependency injection through ordinary Rust mechanisms.
- Strong domain types.
- Deterministic core logic.
- Testable application logic independent of the GUI.
- Side effects isolated at system boundaries.
- Clear ownership of mutable state.

## Discouraged

- Global mutable state.
- Hidden side effects.
- `unwrap()` in recoverable paths.
- Giant stateful components.
- Unnecessary traits.
- Excessive generics.
- Service locators.
- Singleton-style application state.
- Callback spaghetti.
- Framework-driven architecture.
- Abstractions created before they are needed.

## Forbidden by Default

```text
unsafe
global mutable state
panic-based application control flow
runtime-heavy dependencies
unnecessary async
unnecessary background tasks
large third-party frameworks
```

Exceptions require a concrete reason documented near the decision.

---

# Architecture Direction

The preferred architecture is:

```text
                 ┌───────────────┐
                 │     Input     │
                 └───────┬───────┘
                         │
                         ▼
                ┌─────────────────┐
                │  State / Events │
                └────────┬────────┘
                         │
                         ▼
                ┌─────────────────┐
                │ Application     │
                │ Logic           │
                └────────┬────────┘
                         │
             ┌───────────┴───────────┐
             ▼                       ▼
      ┌─────────────┐        ┌─────────────┐
      │ Markdown    │        │ Document    │
      │ Engine      │        │ Model       │
      └─────────────┘        └─────────────┘
             │                       │
             └───────────┬───────────┘
                         ▼
                ┌─────────────────┐
                │ Rendering / UI  │
                └─────────────────┘

External effects remain at the boundaries:
filesystem / clipboard / window / OS
```

The architecture should remain small and understandable. Do not introduce layers solely because a larger application might eventually need them.

---

# Implementation Order

## Phase 1: Window

- Create window.
- Draw pixels.
- Handle keyboard input.
- Implement scrolling.

## Phase 2: Text

- Add tiny bitmap font.
- Render text.
- Cursor.
- Selection.
- Basic editing.

## Phase 3: Markdown

- Block parser.
- Inline parser.
- Markdown renderer.
- View/edit toggle.

## Phase 4: Files

- Open `.md`.
- Save.
- Command-line file argument.

Example:

```bash
mimd README.md
```

or:

```bash
mdvue README.md
```

## Phase 5: Size Optimization

Measure:

```text
Binary size
Startup time
Memory usage
Dependency size
```

Then remove anything unnecessary.

Optimization is part of the project, not a final ritual performed after the dependency graph has already become a small ecosystem.

---

# CLI

Keep the CLI minimal.

Initial form:

```bash
mimd README.md
mimd notes.md
```

or:

```bash
mdvue README.md
mdvue notes.md
```

Potential future options:

```bash
mimd --view README.md
mimd --edit README.md
```

or the equivalent using `mdvue`.

Do not build a large command-line interface for a tiny GUI tool.

---

# Non-Goals

This project should **not** become:

- A full Markdown editor.
- An IDE.
- A note-taking application.
- An Obsidian replacement.
- A plugin platform.
- A browser.
- A document management system.
- A full CommonMark implementation.
- A general-purpose text editor.

The project succeeds by staying narrow.

---

# Success Criteria

The project is successful if:

1. A Markdown file opens almost instantly.
2. Common simple Markdown renders correctly.
3. Basic editing works reliably.
4. Files can be saved.
5. The application feels responsive.
6. The release binary stays around or below **2 MB**.
7. The dependency graph remains small.
8. The core logic is independently testable.
9. The implementation remains small enough to understand in one sitting.
10. Effects, state, failures, and dependencies remain explicit.

---

# Future Ideas

Only consider these after the minimal version is complete:

- Search
- Go-to-line
- Recent files
- Dark/light mode
- Better Markdown typography
- Find/replace
- Multiple files
- Basic syntax coloring
- Native file associations

These are deliberately deferred.

Feature creep is how a 2 MB project becomes a 180 MB "productivity platform."

---

# Guiding Principles

The project should follow these principles:

> **Make effects explicit.**

> **Keep the core pure.**

> **Make state understandable.**

> **Make failures explicit and typed.**

> **Keep dependencies small.**

> **Keep abstractions justified.**

> **Optimize for simplicity before cleverness.**

> **Preserve the ≤ 2 MB goal unless a concrete trade-off is consciously accepted.**

The goal is not to imitate TypeScript or the Effect ecosystem literally.

The goal is to bring the useful **Effect-style mental model** into idiomatic Rust while preserving the project's primary constraint:

```text
Tiny binary.
Tiny dependency graph.
Tiny architecture.
Predictable behavior.
```

---

# Project Identity

```text
MIMD / MDVue
Tiny native Markdown viewer + editor
Rust
Linux
≤ 2 MB target binary
Minimal Markdown subset
Minimal dependencies
Pure core / explicit effects
Strong domain types
Typed errors
Explicit state transitions
GUI-independent core tests
```

This document is the source of truth for the project idea until implementation begins.
