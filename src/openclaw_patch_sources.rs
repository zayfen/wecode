const NATIVE_APPROVAL_JS: &str = include_str!("openclaw_patches/native_approval.js");

pub fn native_approval_patch(name: &str) -> &'static str {
    patch_segment(NATIVE_APPROVAL_JS, name)
}

fn patch_segment(source: &'static str, name: &str) -> &'static str {
    let start_marker = format!("// @wecode-patch {name}\n");
    let end_marker = format!("// @wecode-patch-end {name}");
    let start = source
        .find(&start_marker)
        .unwrap_or_else(|| panic!("missing OpenClaw JS patch start marker: {name}"))
        + start_marker.len();
    let relative_end = source[start..]
        .find(&end_marker)
        .unwrap_or_else(|| panic!("missing OpenClaw JS patch end marker: {name}"));
    source[start..start + relative_end].trim_end_matches('\n')
}
