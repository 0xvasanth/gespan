//! JSON with comments and trailing commas, as VS Code and Zed write it.

/// The text as plain JSON.
pub(crate) struct Stripped {
    pub json: String,
    /// Saving would lose them, so writers must edit the file in place instead.
    // `write.rs`, which lands later, is its first reader outside the tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub had_comments: bool,
}

/// Removes `//` and `/* */` comments and trailing commas outside strings.
/// Comments become spaces, so line numbers in parse errors still match.
pub(crate) fn strip(text: &str) -> Stripped {
    let mut json = String::with_capacity(text.len());
    let mut had_comments = false;
    let mut chars = text.chars().peekable();
    // Where a comma was kept, while only blanks and comments follow it.
    let mut pending_comma: Option<usize> = None;
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                pending_comma = None;
                json.push(c);
                while let Some(c) = chars.next() {
                    json.push(c);
                    match c {
                        '\\' => json.extend(chars.next()),
                        '"' => break,
                        _ => {}
                    }
                }
            }
            '/' if chars.peek() == Some(&'/') => {
                had_comments = true;
                for c in chars.by_ref() {
                    if c == '\n' {
                        json.push(c);
                        break;
                    }
                    json.push(' ');
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                had_comments = true;
                chars.next();
                json.push_str("  ");
                let mut last = '\0';
                for c in chars.by_ref() {
                    json.push(if c == '\n' { c } else { ' ' });
                    if last == '*' && c == '/' {
                        break;
                    }
                    last = c;
                }
            }
            ',' => {
                pending_comma = Some(json.len());
                json.push(c);
            }
            '}' | ']' => {
                if let Some(at) = pending_comma.take() {
                    json.replace_range(at..at + 1, " ");
                }
                json.push(c);
            }
            c if c.is_whitespace() => json.push(c),
            c => {
                pending_comma = None;
                json.push(c);
            }
        }
    }
    Stripped { json, had_comments }
}
