use super::wrapping::push_wrapped_multiline_text;
pub(crate) fn push_inline_section(rendered: &mut String, heading: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }

    ensure_section_gap(rendered);
    rendered.push_str(heading);
    rendered.push(' ');
    rendered.push_str(value);
    rendered.push_str("\n\n");
}

pub(crate) fn push_section(rendered: &mut String, heading: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }

    ensure_section_gap(rendered);
    rendered.push_str(heading);
    rendered.push('\n');
    push_wrapped_multiline_text(rendered, value);
    rendered.push('\n');
}

fn ensure_section_gap(rendered: &mut String) {
    if rendered.is_empty() || rendered.ends_with("\n\n") {
        return;
    }
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    rendered.push('\n');
}
