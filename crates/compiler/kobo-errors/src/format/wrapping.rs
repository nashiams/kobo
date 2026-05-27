const HUMAN_GUIDANCE_WIDTH: usize = 100;
pub(super) fn push_wrapped_multiline_text(rendered: &mut String, value: &str) {
    for (index, line) in value.lines().enumerate() {
        if index > 0 {
            rendered.push('\n');
        }

        if line.trim().is_empty() {
            continue;
        }

        push_wrapped_text(rendered, "  ", "  ", line.trim());
    }
}

pub(super) fn push_wrapped_text(
    rendered: &mut String,
    prefix: &str,
    continuation: &str,
    value: &str,
) {
    let mut line = prefix.to_owned();
    for word in value.split_whitespace() {
        let separator_width = usize::from(!line.ends_with(' '));
        let projected_width = line.chars().count() + separator_width + word.chars().count();
        if projected_width > HUMAN_GUIDANCE_WIDTH && line.trim() != prefix.trim() {
            rendered.push_str(line.trim_end());
            rendered.push('\n');
            line.clear();
            line.push_str(continuation);
            line.push_str(word);
        } else {
            if !line.ends_with(' ') {
                line.push(' ');
            }
            line.push_str(word);
        }
    }
    rendered.push_str(line.trim_end());
}
