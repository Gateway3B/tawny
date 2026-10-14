use dioxus::prelude::*;

/// A piece of a description: plain text, a web link, or a time like `1:23:45`
/// that seeks the player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Piece {
    Text(String),
    Link(String),
    Time { label: String, seconds: u64 },
}

fn is_url_end(c: char) -> bool {
    c.is_whitespace() || c == '<' || c == '>' || c == '"'
}

/// `h:mm:ss` or `m:ss` at the start of `s`; returns its length and seconds.
fn time_at(s: &str) -> Option<(usize, u64)> {
    let mut parts: Vec<u64> = Vec::new();
    let mut end = 0;
    let bytes = s.as_bytes();
    loop {
        let start = end;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        let digits = end - start;
        // Only the first group may be one digit; the rest are minutes and
        // seconds, always two.
        if digits == 0 || digits > 2 || (!parts.is_empty() && digits != 2) {
            return None;
        }
        parts.push(s[start..end].parse().ok()?);
        if parts.len() < 3
            && bytes.get(end) == Some(&b':')
            && bytes.get(end + 1).is_some_and(u8::is_ascii_digit)
        {
            end += 1;
        } else {
            break;
        }
    }
    if parts.len() < 2 || parts[1..].iter().any(|&p| p >= 60) {
        return None;
    }
    let seconds = parts.iter().fold(0, |total, &p| total * 60 + p);
    Some((end, seconds))
}

pub(super) fn linkify(text: &str) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut plain = String::new();
    let mut previous: Option<char> = None;
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if rest.starts_with("http://") || rest.starts_with("https://") {
            let mut end = rest.find(is_url_end).unwrap_or(rest.len());
            // Prose punctuation after a link is not part of it.
            while let Some(last) = rest[..end].chars().next_back() {
                let unbalanced_paren = last == ')'
                    && rest[..end].matches('(').count() >= rest[..end].matches(')').count();
                if matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | ']' | '\'')
                    || (last == ')' && !unbalanced_paren)
                {
                    end -= last.len_utf8();
                } else {
                    break;
                }
            }
            if end > "https://".len() - 1 {
                if !plain.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut plain)));
                }
                pieces.push(Piece::Link(rest[..end].to_string()));
                previous = rest[..end].chars().next_back();
                rest = &rest[end..];
                continue;
            }
        }
        if c.is_ascii_digit() && !previous.is_some_and(|p| p.is_alphanumeric() || p == ':') {
            if let Some((len, seconds)) = time_at(rest) {
                if !plain.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut plain)));
                }
                pieces.push(Piece::Time {
                    label: rest[..len].to_string(),
                    seconds,
                });
                previous = rest[..len].chars().next_back();
                rest = &rest[len..];
                continue;
            }
        }
        plain.push(c);
        previous = Some(c);
        rest = &rest[c.len_utf8()..];
    }
    if !plain.is_empty() {
        pieces.push(Piece::Text(plain));
    }
    pieces
}

/// Description text with its links and time codes made tappable.
#[component]
pub(super) fn Linkified(text: String, on_seek: EventHandler<u64>) -> Element {
    let pieces = linkify(&text);
    rsx! {
        for piece in pieces {
            match piece {
                Piece::Text(text) => rsx! { "{text}" },
                Piece::Link(url) => rsx! {
                    a {
                        href: "{url}",
                        target: "_blank",
                        rel: "noopener noreferrer",
                        style: "color: var(--g3-color-accent); text-decoration: underline;",
                        "{url}"
                    }
                },
                Piece::Time { label, seconds } => rsx! {
                    a {
                        href: "#",
                        style: "color: var(--g3-color-accent);",
                        onclick: move |event| {
                            event.prevent_default();
                            on_seek.call(seconds);
                        },
                        "{label}"
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_links_and_times() {
        let pieces =
            linkify("0:00 Intro\n12:34 Part (see https://example.com/a?b=1). Done 1:02:03");
        assert_eq!(
            pieces[0],
            Piece::Time {
                label: "0:00".into(),
                seconds: 0
            }
        );
        assert!(pieces.contains(&Piece::Time {
            label: "12:34".into(),
            seconds: 754
        }));
        assert!(pieces.contains(&Piece::Link("https://example.com/a?b=1".into())));
        assert!(pieces.contains(&Piece::Time {
            label: "1:02:03".into(),
            seconds: 3723
        }));
    }

    #[test]
    fn ignores_non_times() {
        let pieces = linkify("version 1.2:3 at 10:75 or 123:45");
        assert_eq!(
            pieces,
            vec![Piece::Text("version 1.2:3 at 10:75 or 123:45".into())]
        );
    }

    #[test]
    fn times_inside_links_stay_in_the_link() {
        let pieces = linkify("https://youtu.be/x?t=1:23");
        assert_eq!(
            pieces,
            vec![Piece::Link("https://youtu.be/x?t=1:23".into())]
        );
    }
}
