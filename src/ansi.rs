pub(crate) fn color_code(color: bool, code: &'static str) -> &'static str {
    if color { code } else { "" }
}

pub(crate) fn color_reset(color: bool) -> &'static str {
    if color { "\x1b[0m" } else { "" }
}
