//! System font resolution and glyph rasterization.
//!
//! No font is bundled. Fonts are resolved through fontconfig (which honours
//! the user's font configuration) and rasterized with FreeType, both bound
//! at runtime via `dlopen`. Nothing is linked at build time, so the binary
//! carries no font data and no font engine.
//!
//! This module is the only place that knows about concrete font families.
//! Everything else speaks in [`Role`]s.
//!
//! # Why `unsafe` appears here
//!
//! The project rules forbid `unsafe` by default and require exceptions to be
//! documented. This module is that exception, and the reason is precisely the
//! "prefer platform capabilities" rule: rather than embedding a font or
//! vendoring a rasteriser, we call the FreeType and fontconfig libraries the
//! operating system already ships.
//!
//! Both are opened with `dlopen` at runtime, so:
//!
//!   * the binary links against neither (no `libfreetype` in `ldd`),
//!   * no font data or font engine is compiled in,
//!   * every symbol is resolved once at startup and fails cleanly with a
//!     typed [`FontError`] if the library or symbol is absent.
//!
//! All `unsafe` is confined to this file; `main.rs` contains none.

use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_long, c_uint, c_ulong, c_void};

// ---------- typography roles ----------

/// Semantic typography roles. The renderer asks for a role; it never names a family.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Role {
    /// System proportional / UI font: paragraphs, headings, lists, chrome.
    Body,
    /// System proportional / UI font, bold weight.
    BodyBold,
    /// System proportional / UI font, italic weight.
    BodyItalic,
    /// System proportional / UI font, bold italic weight.
    BodyBoldItalic,
    /// System monospace font: inline code and code-oriented content.
    Code,
    /// System monospace font, bold weight. Part of the weight axis for
    /// completeness; the renderer does not yet need bold monospace.
    #[allow(dead_code)]
    CodeBold,
}

impl Role {
    /// Which family this role draws from. Weight never changes family.
    fn family(self) -> Family {
        match self {
            Role::Code | Role::CodeBold => Family::Code,
            _ => Family::Body,
        }
    }

    /// fontconfig pattern suffix selecting this role's weight.
    fn style_suffix(self) -> &'static str {
        match self {
            Role::Body | Role::Code => ":",
            Role::BodyBold | Role::CodeBold => ":bold",
            Role::BodyItalic => ":italic",
            Role::BodyBoldItalic => ":bold:italic",
        }
    }
}

/// The two families mimd draws from.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Family {
    Body,
    Code,
}

// ---------- fontconfig FFI ----------

const FC_FILE: &[u8] = b"file\0";
const FC_FAMILY_SYSTEM_UI: &str = "system-ui";

type FcChar8 = u8;
type FcPattern = c_void;
type FcConfig = c_void;

/// Frees a fontconfig pattern on drop, using the runtime-resolved destructor.
struct PatternGuard {
    pat: *mut FcPattern,
    destroy: Option<unsafe extern "C" fn(*mut FcPattern)>,
}

impl Drop for PatternGuard {
    fn drop(&mut self) {
        if let Some(d) = self.destroy {
            unsafe { d(self.pat) }
        }
    }
}

// ---------- FreeType FFI ----------

#[allow(non_camel_case_types)]
type FT_Library = c_void;
#[allow(non_camel_case_types)]
type FT_Face = c_void;

const FT_LOAD_RENDER: c_int = 1 << 2;

/// Mirrors `FT_Bitmap` — the only FreeType struct we read fields from.
#[repr(C)]
#[derive(Clone, Copy)]
struct FtBitmap {
    rows: c_uint,
    width: c_uint,
    pitch: c_int,
    buffer: *const u8,
    num_grays: u16,
    pixel_mode: u8,
    palette_mode: u8,
    palette: *mut c_void,
}

/// Field offsets we need inside `FT_GlyphSlotRec` (see freetype.h).
///
/// Only the prefix up to `bitmap_top` is declared; the struct is opaque past
/// that point and we never touch it.
#[repr(C)]
struct FtGlyphSlot {
    library: *mut c_void,
    face: *mut c_void,
    next: *mut c_void,
    glyph_index: c_uint,
    generic_data: *mut c_void,
    generic_finalizer: *mut c_void,
    metrics: [c_long; 8],
    linear_hori_advance: c_long,
    linear_vert_advance: c_long,
    advance_x: c_long,
    advance_y: c_long,
    format: c_uint,
    bitmap: FtBitmap,
    bitmap_left: c_int,
    bitmap_top: c_int,
}

/// Field offsets we need inside `FT_FaceRec` (see freetype.h).
#[repr(C)]
struct FtFace {
    num_faces: c_long,
    face_index: c_long,
    face_flags: c_long,
    style_flags: c_long,
    num_glyphs: c_long,
    family_name: *const c_char,
    style_name: *const c_char,
    num_fixed_sizes: c_int,
    available_sizes: *mut c_void,
    num_charmaps: c_int,
    charmaps: *mut c_void,
    generic_data: *mut c_void,
    generic_finalizer: *mut c_void,
    bbox: [c_long; 4],
    units_per_em: u16,
    ascender: i16,
    descender: i16,
    height: i16,
    max_advance_width: i16,
    max_advance_height: i16,
    underline_position: i16,
    underline_thickness: i16,
    glyph: *mut FtGlyphSlot,
    size: *mut c_void,
    charmap: *mut c_void,
}

// ---------- dlopen ----------

extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

const RTLD_NOW: c_int = 2;
const RTLD_GLOBAL: c_int = 0x100;

/// Runtime-resolved symbol table. Nothing here is linked at build time.
struct Lib {
    handle: *mut c_void,
}

impl Lib {
    /// # Safety
    /// `name` must be a symbol exported by this library.
    unsafe fn sym(&self, name: &str) -> Result<*mut c_void, FontError> {
        let c = CString::new(name).map_err(|_| FontError::Symbol(name.to_string()))?;
        let p = dlsym(self.handle, c.as_ptr());
        if p.is_null() {
            Err(FontError::Symbol(name.to_string()))
        } else {
            Ok(p)
        }
    }
}

impl Drop for Lib {
    fn drop(&mut self) {
        // Deliberately leaked: fontconfig and FreeType are process-lifetime
        // singletons and we must not unload them out from under cached faces.
    }
}

/// Everything we call into, resolved once at startup.
struct Api {
    fc_init: unsafe extern "C" fn() -> c_int,
    fc_config_get_current: unsafe extern "C" fn() -> *mut FcConfig,
    fc_config_substitute: unsafe extern "C" fn(*mut FcConfig, *mut FcPattern, c_int),
    fc_default_substitute: unsafe extern "C" fn(*mut FcPattern),
    fc_font_match: unsafe extern "C" fn(*mut FcConfig, *mut FcPattern, *mut c_int) -> *mut FcPattern,
    fc_pattern_create: unsafe extern "C" fn() -> *mut FcPattern,
    fc_pattern_destroy: unsafe extern "C" fn(*mut FcPattern),
    fc_name_parse: unsafe extern "C" fn(*const FcChar8) -> *mut FcPattern,
    fc_pattern_get_string: unsafe extern "C" fn(
        *const FcPattern,
        *const c_char,
        c_int,
        *mut *mut FcChar8,
    ) -> c_int,
    ft_init_freetype: unsafe extern "C" fn(*mut *mut FT_Library) -> c_int,
    ft_new_face: unsafe extern "C" fn(
        *mut FT_Library,
        *const c_char,
        c_long,
        *mut *mut FT_Face,
    ) -> c_int,
    ft_set_pixel_sizes: unsafe extern "C" fn(*mut FT_Face, c_uint, c_uint) -> c_int,
    ft_load_char: unsafe extern "C" fn(*mut FT_Face, c_ulong, c_int) -> c_int,
    ft_get_char_index: unsafe extern "C" fn(*mut FT_Face, c_ulong) -> c_uint,
    ft_done_face: unsafe extern "C" fn(*mut FT_Face) -> c_int,
}

// ---------- errors ----------

#[derive(Debug)]
pub enum FontError {
    /// Neither FreeType nor fontconfig could be loaded.
    LibraryMissing { lib: String, message: String },
    Symbol(String),
    /// No font on the system matched the requested role.
    NoFontForRole(&'static str),
    /// FreeType refused to open a font file.
    FaceLoad { path: String, code: c_int },
}

impl std::fmt::Display for FontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FontError::LibraryMissing { lib, message } => {
                write!(f, "cannot load {lib}: {message}")
            }
            FontError::Symbol(s) => write!(f, "missing system symbol {s}"),
            FontError::NoFontForRole(r) => write!(f, "no system font found for role {r}"),
            FontError::FaceLoad { path, code } => {
                write!(f, "FreeType cannot open {path} (error {code})")
            }
        }
    }
}

impl FontError {
    fn missing(lib: &str) -> Self {
        FontError::LibraryMissing {
            lib: lib.to_string(),
            message: "not installed on this system".into(),
        }
    }
}

// ---------- glyphs ----------

/// A rasterized glyph: an 8-bit coverage mask plus its placement.
#[derive(Clone)]
pub struct Glyph {
    /// Coverage mask, row-major, `width` bytes per row.
    pub mask: Vec<u8>,
    pub width: u32,
    #[allow(dead_code)]
    pub height: u32,
    /// Offset from the pen position to the left edge of the mask.
    pub bearing_x: i32,
    /// Offset from the baseline up to the top row of the mask.
    pub bearing_y: i32,
    /// How far the pen advances after drawing.
    pub advance_x: i32,
}

impl Glyph {
    fn blank(advance_x: i32) -> Self {
        Glyph { mask: Vec::new(), width: 0, height: 0, bearing_x: 0, bearing_y: 0, advance_x }
    }

    pub fn is_blank(&self) -> bool {
        self.mask.is_empty()
    }
}

/// One resolved typeface, with its rasterized glyph cache.
pub struct Font {
    face: *mut FT_Face,
    /// Shared FreeType library. Never freed: faces may outlive it in the
    /// fallback cache and FreeType handles are process-lifetime.
    #[allow(dead_code)]
    library: *mut FT_Library,
    /// Resolved family name, for diagnostics and `--font` reporting.
    #[allow(dead_code)]
    pub family: String,
    pub size: u16,
    /// Distance from the baseline to the top of the line box, in pixels.
    pub ascent: i32,
    /// Distance from the baseline down, in pixels.
    pub descent: i32,
    cache: HashMap<u32, Glyph>,
    api: *const Api,
}

impl Font {
    pub fn line_height(&self) -> u32 {
        (self.ascent + self.descent).max(1) as u32
    }

    /// A second handle on the same already-open face, with an empty glyph cache.
    ///
    /// Used only as a last-resort fallback when a styled face cannot be opened,
    /// so a missing bold cut degrades to regular text instead of no text. The
    /// face is intentionally not duplicated: it is process-lifetime.
    fn clone_shallow(&self) -> Font {
        Font {
            face: self.face,
            library: self.library,
            family: self.family.clone(),
            size: self.size,
            ascent: self.ascent,
            descent: self.descent,
            cache: HashMap::new(),
            api: self.api,
        }
    }

    /// Rasterize `ch`, or return a blank glyph with the right advance.
    pub fn glyph(&mut self, ch: char) -> Glyph {
        let cp = ch as u32;
        if let Some(g) = self.cache.get(&cp) {
            return g.clone();
        }
        let api = unsafe { &*self.api };
        unsafe {
            let idx = (api.ft_get_char_index)(self.face, cp as c_ulong);
            let g = if idx == 0 {
                Glyph::blank(self.space_advance())
            } else if (api.ft_load_char)(self.face, cp as c_ulong, FT_LOAD_RENDER) != 0 {
                Glyph::blank(self.space_advance())
            } else {
                let rec = self.face as *const FtFace;
                let slot = (*rec).glyph;
                let bmp = (*slot).bitmap;
                let adv = ((*slot).advance_x >> 6) as i32;
                // pitch may be negative; copy row by row to normalise it.
                let mut mask = Vec::with_capacity((bmp.width * bmp.rows) as usize);
                if !bmp.buffer.is_null() && bmp.pixel_mode == 2 {
                    for row in 0..bmp.rows as isize {
                        let start = if bmp.pitch >= 0 {
                            row * bmp.pitch as isize
                        } else {
                            (bmp.rows as isize - 1 - row) * (-bmp.pitch) as isize
                        };
                        let src = bmp.buffer.offset(start) as *const u8;
                        for col in 0..bmp.width as isize {
                            mask.push(*src.offset(col));
                        }
                    }
                }
                Glyph {
                    mask,
                    width: bmp.width,
                    height: bmp.rows,
                    bearing_x: (*slot).bitmap_left,
                    bearing_y: (*slot).bitmap_top,
                    advance_x: adv,
                }
            };
            self.cache.insert(cp, g.clone());
            g
        }
    }

    fn space_advance(&mut self) -> i32 {
        self.glyph_raw(' ').map(|g| g.advance_x).unwrap_or(0)
    }

    fn glyph_raw(&mut self, ch: char) -> Option<Glyph> {
        let cp = ch as u32;
        if let Some(g) = self.cache.get(&cp) {
            return Some(g.clone());
        }
        let api = unsafe { &*self.api };
        unsafe {
            let idx = (api.ft_get_char_index)(self.face, ch as u32 as c_ulong);
            if idx == 0 {
                return None;
            }
            if (api.ft_load_char)(self.face, ch as u32 as c_ulong, FT_LOAD_RENDER) != 0 {
                return None;
            }
            let rec = self.face as *const FtFace;
            let slot = (*rec).glyph;
            let bmp = (*slot).bitmap;
            let mut mask = Vec::new();
            if !bmp.buffer.is_null() && bmp.pixel_mode == 2 {
                for row in 0..bmp.rows as isize {
                    let start = if bmp.pitch >= 0 {
                        row * bmp.pitch as isize
                    } else {
                        (bmp.rows as isize - 1 - row) * (-bmp.pitch) as isize
                    };
                    let src = bmp.buffer.offset(start) as *const u8;
                    for col in 0..bmp.width as isize {
                        mask.push(*src.offset(col));
                    }
                }
            }
            let g = Glyph {
                mask,
                width: bmp.width,
                height: bmp.rows,
                bearing_x: (*slot).bitmap_left,
                bearing_y: (*slot).bitmap_top,
                advance_x: ((*slot).advance_x >> 6) as i32,
            };
            self.cache.insert(cp, g.clone());
            Some(g)
        }
    }

    /// Horizontal pen advance for `s`.
    pub fn measure(&mut self, s: &str) -> i32 {
        s.chars().map(|c| self.glyph(c).advance_x).sum()
    }

    /// Pen x for the caret sitting before character index `char_idx`.
    /// Pen x for the caret sitting before character index `char_idx`.
    #[allow(dead_code)]
    pub fn caret_x(&mut self, s: &str, char_idx: usize) -> i32 {
        s.chars().take(char_idx).map(|c| self.glyph(c).advance_x).sum()
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        let api = unsafe { &*self.api };
        unsafe {
            (api.ft_done_face)(self.face);
        }
    }
}

/// Resolved typography for the whole application.
///
/// Holds one face per [`Role`] plus per-glyph fallback faces, so Unicode
/// coverage comes from whatever the system provides.
pub struct Fonts {
    pub body: Font,
    pub code: Font,
    /// Face per role at the base size, for weights beyond Body/Code.
    base_faces: HashMap<Role, Font>,
    body_path: String,
    code_path: String,
    /// `--font` / `--mono` overrides, kept so styled faces match the override.
    body_family_override: Option<String>,
    code_family_override: Option<String>,
    /// Resolved file per (family, style), so a weight is asked of fontconfig once.
    style_paths: HashMap<Role, String>,
    base_size: u16,
    library: *mut FT_Library,
    /// Rasterized glyphs for codepoints the primary faces lack.
    fallback: HashMap<u32, Glyph>,
    /// Codepoint eviction order for `fallback`.
    fallback_order: Vec<u32>,
    /// Faces kept alive so fallback glyphs stay valid.
    fallback_faces: Vec<Font>,
    /// Extra faces opened at non-base sizes (headings), keyed by role and size.
    sized: HashMap<(Role, u16), Font>,
    api: Box<Api>,
    /// Keeps fontconfig initialised for the process lifetime.
    _fc: PatternGuard,
    _fcl: Lib,
    _ftl: Lib,
}

const MAX_FALLBACK_FACES: usize = 16;

impl Fonts {
    /// Resolve the system fonts for both roles.
    ///
    /// `body_family` / `code_family` are resolved through fontconfig; passing
    /// `None` for each asks fontconfig for its configured default (honouring
    /// the user's font settings).
    pub fn new(
        body_family: Option<&str>,
        code_family: Option<&str>,
        size: u16,
    ) -> Result<Fonts, FontError> {
        let fcl = unsafe { load_lib("libfontconfig.so.1").or_else(|_| load_lib("libfontconfig.so"))? };
        let ftl = unsafe { load_lib("libfreetype.so.6").or_else(|_| load_lib("libfreetype.so"))? };

        let api = Api {
            fc_init: unsafe { std::mem::transmute(fcl.sym("FcInit")?) },
            fc_config_get_current: unsafe { std::mem::transmute(fcl.sym("FcConfigGetCurrent")?) },
            fc_config_substitute: unsafe { std::mem::transmute(fcl.sym("FcConfigSubstitute")?) },
            fc_default_substitute: unsafe { std::mem::transmute(fcl.sym("FcDefaultSubstitute")?) },
            fc_font_match: unsafe { std::mem::transmute(fcl.sym("FcFontMatch")?) },
            fc_pattern_create: unsafe { std::mem::transmute(fcl.sym("FcPatternCreate")?) },
            fc_pattern_destroy: unsafe { std::mem::transmute(fcl.sym("FcPatternDestroy")?) },
            fc_pattern_get_string: unsafe {
                std::mem::transmute(fcl.sym("FcPatternGetString")?)
            },
            fc_name_parse: unsafe { std::mem::transmute(fcl.sym("FcNameParse")?) },
            ft_init_freetype: unsafe { std::mem::transmute(ftl.sym("FT_Init_FreeType")?) },
            ft_new_face: unsafe { std::mem::transmute(ftl.sym("FT_New_Face")?) },
            ft_set_pixel_sizes: unsafe { std::mem::transmute(ftl.sym("FT_Set_Pixel_Sizes")?) },
            ft_load_char: unsafe { std::mem::transmute(ftl.sym("FT_Load_Char")?) },
            ft_get_char_index: unsafe { std::mem::transmute(ftl.sym("FT_Get_Char_Index")?) },
            ft_done_face: unsafe { std::mem::transmute(ftl.sym("FT_Done_Face")?) },
        };

        unsafe {
            (api.fc_init)();
        }
        let api = Box::new(api);
        let api_ptr = &*api as *const Api;

        let body_path = resolve_font_path(&api, body_family, FC_FAMILY_SYSTEM_UI, ":")?;
        let code_path = resolve_font_path(&api, code_family, "monospace", ":")?;

        // A shared FreeType library for every face we open.
        let mut library: *mut FT_Library = std::ptr::null_mut();
        unsafe {
            if (api.ft_init_freetype)(&mut library) != 0 {
                return Err(FontError::FaceLoad { path: "libfreetype".into(), code: -1 });
            }
        }

        let body = open_face(&api, api_ptr, library, &body_path, size)?;
        let code = open_face(&api, api_ptr, library, &code_path, size)?;

        // Keep one never-null pattern so fontconfig stays initialised.
        let keep = PatternGuard {
            pat: unsafe { (api.fc_pattern_create)() },
            destroy: Some(api.fc_pattern_destroy),
        };

        Ok(Fonts {
            body,
            code,
            base_faces: HashMap::new(),
            body_path,
            code_path,
            body_family_override: body_family.map(String::from),
            code_family_override: code_family.map(String::from),
            style_paths: HashMap::new(),
            base_size: size,
            library,
            fallback: HashMap::new(),
            fallback_order: Vec::new(),
            fallback_faces: Vec::new(),
            sized: HashMap::new(),
            api,
            _fc: keep,
            _fcl: fcl,
            _ftl: ftl,
        })
    }

    /// Line height for `role` at the base size, without opening anything.
    ///
    /// Read-only counterpart to [`Self::line_height_at`] for the hot path
    /// (status bar, caret), where a styled face is already open or the
    /// family's metrics are an acceptable answer.
    pub fn line_height_of(&self, role: Role) -> u32 {
        match role.family() {
            Family::Body => self.body.line_height(),
            Family::Code => self.code.line_height(),
        }
    }

    /// Baseline offset for `role` at the base size, without opening anything.
    pub fn ascent_of(&self, role: Role) -> i32 {
        match role.family() {
            Family::Body => self.body.ascent,
            Family::Code => self.code.ascent,
        }
    }

    pub fn for_role(&mut self, role: Role) -> &mut Font {
        match role {
            Role::Body => &mut self.body,
            Role::Code => &mut self.code,
            other => {
                if !self.base_faces.contains_key(&other) {
                    let f = self.open_role(other, self.base_size);
                    self.base_faces.insert(other, f);
                }
                self.base_faces.get_mut(&other).expect("just inserted")
            }
        }
    }

    /// Resolve the font file for a role's family and weight through fontconfig,
    /// falling back to the family's regular face when no styled face exists.
    fn role_path(&mut self, role: Role) -> String {
        if let Some(p) = self.style_paths.get(&role) {
            return p.clone();
        }
        let (family, suffix) = (role.family(), role.style_suffix());
        let (override_family, generic) = match family {
            Family::Body => (self.body_family_override.clone(), FC_FAMILY_SYSTEM_UI),
            Family::Code => (self.code_family_override.clone(), "monospace"),
        };
        let styled = resolve_font_path(&self.api, override_family.as_deref(), generic, suffix);
        // A system without a bold or italic cut still has to render: fall back
        // to the family's regular face rather than failing the whole document.
        let path = match styled {
            Ok(p) if suffix != ":" => {
                let regular = match family {
                    Family::Body => self.body_path.clone(),
                    Family::Code => self.code_path.clone(),
                };
                if p == regular {
                    regular
                } else {
                    p
                }
            }
            Ok(p) => p,
            Err(_) => match family {
                Family::Body => self.body_path.clone(),
                Family::Code => self.code_path.clone(),
            },
        };
        self.style_paths.insert(role, path.clone());
        path
    }

    /// Open `role`'s face at `size`, on the shared library.
    fn open_role(&mut self, role: Role, size: u16) -> Font {
        let path = self.role_path(role);
        let api_ptr = &*self.api as *const Api;
        open_face(&self.api, api_ptr, self.library, &path, size)
            .unwrap_or_else(|_| match role.family() {
                Family::Body => self.body.clone_shallow(),
                Family::Code => self.code.clone_shallow(),
            })
    }

    /// Face for `role` at an arbitrary point size, opened on demand.
    ///
    /// Sizes other than the base size are rare (headings), so faces are
    /// opened lazily and cached rather than preloading a size ramp.
    pub fn face_sized(&mut self, role: Role, size: u16) -> Result<&mut Font, FontError> {
        if size == self.base_size {
            let f = self.for_role(role);
            return Ok(f);
        }
        if !self.sized.contains_key(&(role, size)) {
            let font = self.open_role(role, size);
            self.sized.insert((role, size), font);
        }
        Ok(self.sized.get_mut(&(role, size)).expect("just inserted"))
    }

    /// Width of `s` in `role` at `size` pixels.
    pub fn measure_at(&mut self, role: Role, size: u16, s: &str) -> i32 {
        match self.face_sized(role, size) {
            Ok(f) => f.measure(s),
            Err(_) => self.for_role(role).measure(s),
        }
    }

    /// Rasterize one char in `role` at `size`, with system fallback.
    ///
    /// At the base size this is exactly `glyph` (fallback included). At
    /// another size we check coverage first and only pay for a fallback face
    /// when the sized face genuinely lacks the glyph.
    pub fn glyph_at(&mut self, role: Role, size: u16, ch: char) -> Glyph {
        if size == self.base_size {
            return self.glyph(role, ch);
        }
        let g = match self.face_sized(role, size) {
            Ok(f) => f.glyph(ch),
            Err(_) => self.for_role(role).glyph(ch),
        };
        if !g.is_blank() || ch == ' ' {
            return g;
        }
        self.fallback_glyph(role, ch).unwrap_or(g)
    }

    /// Line height for `role` at `size`.
    pub fn line_height_at(&mut self, role: Role, size: u16) -> u32 {
        match self.face_sized(role, size) {
            Ok(f) => f.line_height(),
            Err(_) => self.for_role(role).line_height(),
        }
    }

    /// Baseline offset for `role` at `size`.
    pub fn ascent_at(&mut self, role: Role, size: u16) -> i32 {
        match self.face_sized(role, size) {
            Ok(f) => f.ascent,
            Err(_) => self.for_role(role).ascent,
        }
    }

    /// Resolve a glyph for `ch`, falling back to another system font when the
    /// primary face for `role` has no coverage.
    pub fn glyph(&mut self, role: Role, ch: char) -> Glyph {
        let g = self.for_role(role).glyph(ch);
        if !g.is_blank() || ch == ' ' {
            return g;
        }
        self.fallback_glyph(role, ch).unwrap_or(g)
    }

    /// Glyph for `ch` from another system font, cached.
    fn fallback_glyph(&mut self, role: Role, ch: char) -> Option<Glyph> {
        let cp = ch as u32;
        if let Some(g) = self.fallback.get(&cp) {
            return Some(g.clone());
        }
        self.build_fallback(role, ch)
    }

    /// Ask fontconfig for a face that covers `ch`, then rasterize from it.
    fn build_fallback(&mut self, role: Role, ch: char) -> Option<Glyph> {
        // Preserve the primary role's style by asking for the same family
        // class plus a charset containing the missing codepoint.
        let path = self.resolve_for_char(role, ch)?;
        let size = match role.family() {
            Family::Body => self.body.size,
            Family::Code => self.code.size,
        };
        let api_ptr = &*self.api as *const Api;
        let mut library: *mut FT_Library = std::ptr::null_mut();
        unsafe {
            if (self.api.ft_init_freetype)(&mut library) != 0 {
                return None;
            }
        }
        let mut font = open_face(&self.api, api_ptr, library, &path, size).ok()?;
        let g = font.glyph(ch);
        if g.is_blank() {
            return None;
        }
        self.fallback_faces.push(font);
        self.fallback_order.push(ch as u32);
        self.fallback.insert(ch as u32, g.clone());
        if self.fallback_faces.len() > MAX_FALLBACK_FACES {
            self.fallback.remove(&self.fallback_order[0]);
            self.fallback_order.remove(0);
            self.fallback_faces.remove(0);
        }
        Some(g)
    }

    /// Resolve a font file that covers `ch`.
    ///
    /// fontconfig does the work: `:charset=NNNN` asks it for the best
    /// installed font containing that codepoint, honouring the user's
    /// configuration and its own fallback ordering.
    fn resolve_for_char(&self, role: Role, ch: char) -> Option<String> {
        let path = resolve_font_path(&self.api, Some(&charset_query(ch)), "sans-serif", ":").ok()?;
        // Only use it if it genuinely covers the codepoint.
        self.face_covers(&path, role, ch).then_some(path)
    }

    /// Open `path` briefly and ask FreeType whether it has a glyph for `ch`.
    fn face_covers(&self, path: &str, role: Role, ch: char) -> bool {
        let size = match role.family() {
            Family::Body => self.body.size,
            Family::Code => self.code.size,
        };
        let api_ptr = &*self.api as *const Api;
        let mut library: *mut FT_Library = std::ptr::null_mut();
        unsafe {
            if (self.api.ft_init_freetype)(&mut library) != 0 {
                return false;
            }
        }
        let font = match open_face(&self.api, api_ptr, library, path, size) {
            Ok(f) => f,
            Err(_) => return false,
        };
        let idx = unsafe { (self.api.ft_get_char_index)(font.face, ch as u32 as c_ulong) };
        idx != 0
    }
}

/// A fontconfig pattern string selecting any installed font that covers `ch`.
fn charset_query(ch: char) -> String {
    format!(":charset={:x}", ch as u32)
}

/// Ask fontconfig for a font file matching `family`, or its configured default.
///
/// `FcNameParse` is used rather than a raw `FC_FAMILY` string so generic
/// aliases like `monospace` and `system-ui` resolve through the user's font
/// configuration instead of being treated as literal family names.
fn resolve_font_path(
    api: &Api,
    family: Option<&str>,
    generic: &str,
    style: &str,
) -> Result<String, FontError> {
    unsafe {
        let config = (api.fc_config_get_current)();
        if config.is_null() {
            return Err(FontError::missing("fontconfig"));
        }
        let want = format!("{}{}", family.unwrap_or(generic), style);
        let c = CString::new(want).map_err(|_| FontError::NoFontForRole("system default"))?;
        let pat = (api.fc_name_parse)(c.as_ptr() as *const FcChar8);
        if pat.is_null() {
            return Err(FontError::NoFontForRole("system default"));
        }
        let _pat = PatternGuard { pat, destroy: Some(api.fc_pattern_destroy) };

        (api.fc_config_substitute)(config, pat, 0);
        (api.fc_default_substitute)(pat);
        let mut result: c_int = 0;
        let matched = (api.fc_font_match)(config, pat, &mut result);
        if matched.is_null() {
            return Err(FontError::NoFontForRole("system default"));
        }
        let _m = PatternGuard { pat: matched, destroy: Some(api.fc_pattern_destroy) };
        let mut file: *mut FcChar8 = std::ptr::null_mut();
        if (api.fc_pattern_get_string)(matched, FC_FILE.as_ptr() as *const c_char, 0, &mut file) != 0
            || file.is_null()
        {
            return Err(FontError::NoFontForRole("system default"));
        }
        Ok(std::ffi::CStr::from_ptr(file as *const c_char).to_string_lossy().into_owned())
    }
}

fn open_face(
    api: &Api,
    api_ptr: *const Api,
    library: *mut FT_Library,
    path: &str,
    size: u16,
) -> Result<Font, FontError> {
    unsafe {
        let cpath = CString::new(path)
            .map_err(|_| FontError::FaceLoad { path: path.to_string(), code: -1 })?;
        let mut face: *mut FT_Face = std::ptr::null_mut();
        let rc = (api.ft_new_face)(library, cpath.as_ptr(), 0, &mut face);
        if rc != 0 || face.is_null() {
            return Err(FontError::FaceLoad { path: path.to_string(), code: rc });
        }
        if (api.ft_set_pixel_sizes)(face, 0, size as c_uint) != 0 {
            (api.ft_done_face)(face);
            return Err(FontError::FaceLoad { path: path.to_string(), code: -2 });
        }
        let rec = face as *const FtFace;
        let family = if (*rec).family_name.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr((*rec).family_name).to_string_lossy().into_owned()
        };
        let scale = size as f64 / (*rec).units_per_em.max(1) as f64;
        let ascent = ((*rec).ascender as f64 * scale).ceil() as i32;
        let descent = (-((*rec).descender as f64) * scale).ceil() as i32;
        Ok(Font {
            face,
            library,
            family,
            size,
            ascent,
            descent,
            cache: HashMap::new(),
            api: api_ptr,
        })
    }
}

/// # Safety
/// `name` must be a valid library soname.
unsafe fn load_lib(name: &str) -> Result<Lib, FontError> {
    let c = CString::new(name).map_err(|_| FontError::missing(name))?;
    let h = dlopen(c.as_ptr(), RTLD_NOW | RTLD_GLOBAL);
    if h.is_null() {
        Err(FontError::missing(name))
    } else {
        Ok(Lib { handle: h })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Font-dependent assertions must not fail merely because a machine has
    /// different fonts, so these only check that resolution *works at all*.
    fn fonts() -> Option<Fonts> {
        Fonts::new(None, None, 16).ok()
    }

    #[test]
    fn resolves_both_roles_from_the_system() {
        let Some(f) = fonts() else {
            eprintln!("skip: no system fonts available on this machine");
            return;
        };
        assert!(!f.body.family.is_empty(), "body role resolved to no family");
        assert!(!f.code.family.is_empty(), "code role resolved to no family");
        assert!(f.body.line_height() > 0);
        assert!(f.code.line_height() > 0);
    }

    #[test]
    fn rasterizes_ascii_without_panicking() {
        let Some(mut f) = fonts() else {
            return;
        };
        for ch in ' '..='~' {
            let g = f.body.glyph(ch);
            assert!(g.advance_x >= 0, "{ch:?} had negative advance");
            assert_eq!(g.mask.len(), (g.width * g.height) as usize, "{ch:?} mask size");
        }
    }

    #[test]
    fn measure_is_monotonic_and_nonzero() {
        let Some(mut f) = fonts() else {
            return;
        };
        let a = f.body.measure("hello");
        let b = f.body.measure("hello world");
        assert!(a > 0 && b > a, "a={a} b={b}");
    }

    #[test]
    fn caret_x_lands_on_prefix_width() {
        let Some(mut f) = fonts() else {
            return;
        };
        let s = "hello world";
        let mid = f.body.caret_x(s, 5);
        let whole = f.body.measure(s);
        assert!(mid > 0 && mid < whole);
        assert_eq!(mid, f.body.measure("hello"));
    }

    #[test]
    fn missing_glyph_does_not_panic_and_keeps_state_usable() {
        let Some(mut f) = fonts() else {
            return;
        };
        let before = f.body.measure("hello");
        // Unassigned codepoint: must degrade, not crash or corrupt state.
        let g = f.glyph(Role::Body, '\u{10FFFD}');
        assert_eq!(g.mask.len(), (g.width * g.height) as usize);
        // The primary font must still measure identically afterwards.
        assert_eq!(f.body.measure("hello"), before);
    }

    /// Regression guard: the renderer must reach the fallback path at
    /// heading sizes too, not only at the base size.
    #[test]
    fn fallback_works_at_non_base_sizes() {
        let Some(mut f) = fonts() else {
            return;
        };
        for ch in "A日本語Рус".chars() {
            let base = f.glyph_at(Role::Body, 15, ch);
            assert_eq!(base.mask.len(), (base.width * base.height) as usize);
            let big = f.glyph_at(Role::Body, 30, ch);
            assert_eq!(big.mask.len(), (big.width * big.height) as usize);
            // A real glyph must produce ink somewhere.
            let has_ink = big.mask.iter().any(|&c| c > 0);
            assert!(has_ink || ch == ' ', "no ink for {ch:?} at size 30");
        }
    }

    #[test]
    fn fallback_covers_multilingual_text() {
        let Some(mut f) = fonts() else {
            return;
        };
        // Whatever the system has installed, these must not crash.
        for ch in "Aবা日本語Рус".chars() {
            let g = f.glyph(Role::Body, ch);
            assert_eq!(g.mask.len(), (g.width * g.height) as usize, "{ch:?}");
        }
    }
}
