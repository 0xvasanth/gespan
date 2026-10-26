use std::collections::HashMap;

const KNOWN_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "json", "md", "toml", "yaml", "yml", "py", "go", "swift", "kt",
    "css", "html", "sh", "txt", "lock",
];

/// Longest run of spoken words joined into one identifier.
const MAX_JOIN_WORDS: usize = 4;

/// A whitespace-delimited word split into edge punctuation and its core.
struct Word<'a> {
    lead: &'a str,
    core: &'a str,
    trail: &'a str,
}

fn split_word(w: &str) -> Word<'_> {
    let is_core = |c: char| c.is_alphanumeric();
    let start = w.find(is_core).unwrap_or(w.len());
    let end = w
        .rfind(is_core)
        .map(|i| i + w[i..].chars().next().map_or(0, char::len_utf8));
    match end {
        Some(end) => Word {
            lead: &w[..start],
            core: &w[start..end],
            trail: &w[end..],
        },
        None => Word {
            lead: w,
            core: "",
            trail: "",
        },
    }
}

/// Rewrites spoken identifiers in a transcript using the project `terms`.
/// Whitespace runs collapse to single spaces.
pub fn postprocess(text: &str, terms: &[String]) -> String {
    let words = spoken_symbols(text, terms);
    join_terms(&words, terms).join(" ")
}

/// Rule 1: "dot rs", "underscore", "dash dash" and "slash".
fn spoken_symbols(text: &str, terms: &[String]) -> Vec<String> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut out: Vec<String> = Vec::new();
    // The next emitted word attaches to the previous one with no space.
    let mut glue_next = false;
    let mut i = 0;

    while i < tokens.len() {
        let w = split_word(tokens[i]);
        let lower = w.core.to_lowercase();
        let bare = w.lead.is_empty() && w.trail.is_empty();
        let next = tokens.get(i + 1).map(|t| split_word(t));

        if bare && lower == "dot" {
            if let Some(n) = &next {
                let ext = n.core.to_lowercase();
                if n.lead.is_empty() && is_extension(&ext, terms) {
                    let sym = format!(".{ext}{}", n.trail);
                    let attach = out.last().is_some_and(|l| ends_with_core(l)) && !glue_next;
                    if attach {
                        out.last_mut().unwrap().push_str(&sym);
                    } else {
                        emit(&mut out, sym, &mut glue_next);
                    }
                    i += 2;
                    continue;
                }
            }
        }
        if bare && lower == "underscore" {
            let attach = out.last().is_some_and(|l| ends_with_core(l)) && !glue_next;
            if attach {
                out.last_mut().unwrap().push('_');
            } else {
                emit(&mut out, "_".into(), &mut glue_next);
            }
            glue_next = next
                .as_ref()
                .is_some_and(|n| n.lead.is_empty() && !n.core.is_empty());
            i += 1;
            continue;
        }
        if bare && lower == "dash" {
            if let Some(n) = &next {
                if n.lead.is_empty() && n.core.eq_ignore_ascii_case("dash") && n.trail.is_empty() {
                    if let Some(after) = tokens.get(i + 2) {
                        let a = split_word(after);
                        if a.lead.is_empty() && !a.core.is_empty() {
                            emit(&mut out, format!("--{after}"), &mut glue_next);
                            i += 3;
                            continue;
                        }
                    }
                    emit(&mut out, "--".into(), &mut glue_next);
                    i += 2;
                    continue;
                }
            }
        }
        if bare && lower == "slash" {
            let attach = out.last().is_some_and(|l| ends_with_core(l)) && !glue_next;
            let next_ok = next
                .as_ref()
                .is_some_and(|n| n.lead.is_empty() && !n.core.is_empty());
            if attach && next_ok {
                out.last_mut().unwrap().push('/');
                glue_next = true;
                i += 1;
                continue;
            }
        }
        emit(&mut out, tokens[i].to_string(), &mut glue_next);
        i += 1;
    }
    out
}

fn emit(out: &mut Vec<String>, word: String, glue: &mut bool) {
    if *glue {
        if let Some(last) = out.last_mut() {
            last.push_str(&word);
            *glue = false;
            return;
        }
    }
    *glue = false;
    out.push(word);
}

/// True when the word's last char is part of a word (no trailing punctuation).
fn ends_with_core(word: &str) -> bool {
    word.chars().last().is_some_and(|c| c.is_alphanumeric())
}

fn is_extension(ext: &str, terms: &[String]) -> bool {
    if ext.is_empty() {
        return false;
    }
    KNOWN_EXTENSIONS.contains(&ext)
        || terms.iter().any(|t| {
            t.rsplit_once('.')
                .is_some_and(|(_, suffix)| suffix.eq_ignore_ascii_case(ext))
        })
}

/// Rules 2 and 3: join word runs into terms, then fix single-word casing.
fn join_terms(words: &[String], terms: &[String]) -> Vec<String> {
    let index: HashMap<String, &str> = terms
        .iter()
        .map(|t| (t.to_lowercase(), t.as_str()))
        .collect();
    let split: Vec<Word> = words.iter().map(|w| split_word(w)).collect();
    let mut out = Vec::with_capacity(words.len());
    let mut i = 0;

    'outer: while i < words.len() {
        for n in (2..=MAX_JOIN_WORDS).rev() {
            if i + n > words.len() {
                continue;
            }
            let run = &split[i..i + n];
            let clean_interior = run.iter().enumerate().all(|(k, w)| {
                !w.core.is_empty()
                    && (k == 0 || w.lead.is_empty())
                    && (k == n - 1 || w.trail.is_empty())
            });
            if !clean_interior {
                continue;
            }
            let joined: String = run.iter().map(|w| w.core.to_lowercase()).collect();
            if let Some(term) = index.get(&joined) {
                out.push(format!("{}{term}{}", run[0].lead, run[n - 1].trail));
                i += n;
                continue 'outer;
            }
        }
        let w = &split[i];
        match index.get(&w.core.to_lowercase()) {
            Some(term) if !w.core.is_empty() => out.push(format!("{}{term}{}", w.lead, w.trail)),
            _ => out.push(words[i].clone()),
        }
        i += 1;
    }
    out
}
