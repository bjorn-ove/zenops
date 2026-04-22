//! ANSI color helpers shared by the human-facing renderers (`TerminalRenderer`
//! in `output.rs` and the `pkg_list` command). When `on` is false every method
//! returns the empty string, so callers can unconditionally interpolate the
//! styles into their format strings.

pub(crate) struct Styler {
    pub on: bool,
}

impl Styler {
    pub const BOLD: &'static str = "\x1b[1m";
    pub const DIM: &'static str = "\x1b[2m";
    /// Stack the `\x1b[2m` faint modifier on top of a light grey (256-color
    /// level 248). The faint modifier lets the terminal apply its own
    /// theme-aware fade to the light grey fg, producing a "double-faded
    /// white" that reads as *less white* than plain `DIM` — rather than a
    /// solid darker grey, which reads as a different color rather than
    /// dimmer text. Used for the shared `~/.config/zenops` prefix on
    /// right-side paths so the identifying tail stands out.
    pub const EXTRA_DIM: &'static str = "\x1b[2;38;5;248m";
    pub const RED: &'static str = "\x1b[31m";
    pub const GREEN: &'static str = "\x1b[32m";
    pub const YELLOW: &'static str = "\x1b[33m";
    pub const MAGENTA: &'static str = "\x1b[35m";
    pub const CYAN: &'static str = "\x1b[36m";
    pub const BOLD_YELLOW: &'static str = "\x1b[1;33m";
    pub const RESET: &'static str = "\x1b[0m";

    pub fn new(on: bool) -> Self {
        Self { on }
    }

    fn code(&self, code: &'static str) -> &'static str {
        if self.on { code } else { "" }
    }

    pub fn bold(&self) -> &'static str {
        self.code(Self::BOLD)
    }
    pub fn dim(&self) -> &'static str {
        self.code(Self::DIM)
    }
    pub fn extra_dim(&self) -> &'static str {
        self.code(Self::EXTRA_DIM)
    }
    pub fn red(&self) -> &'static str {
        self.code(Self::RED)
    }
    pub fn green(&self) -> &'static str {
        self.code(Self::GREEN)
    }
    pub fn yellow(&self) -> &'static str {
        self.code(Self::YELLOW)
    }
    pub fn magenta(&self) -> &'static str {
        self.code(Self::MAGENTA)
    }
    pub fn cyan(&self) -> &'static str {
        self.code(Self::CYAN)
    }
    pub fn bold_yellow(&self) -> &'static str {
        self.code(Self::BOLD_YELLOW)
    }
    pub fn reset(&self) -> &'static str {
        self.code(Self::RESET)
    }
}

pub(crate) fn color_code(color: bool, code: &'static str) -> &'static str {
    if color { code } else { "" }
}

pub(crate) fn color_reset(color: bool) -> &'static str {
    if color { Styler::RESET } else { "" }
}
