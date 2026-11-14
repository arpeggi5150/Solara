//! A small XML reader for score files: elements, attributes, text and CDATA into a tree.
//!
//! The input is hostile. Depth, node count and total size are bounded; comments, processing
//! instructions and a DOCTYPE are skipped; the five named entities and numeric character
//! references are decoded, and anything malformed is an error, never a panic.

/// Deepest nesting accepted.
const MAX_DEPTH: usize = 64;
/// Most elements in one document.
const MAX_NODES: usize = 4_000_000;

/// One element: its name, attributes, text (all text and CDATA directly inside it, joined) and
/// children in document order.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Element {
    pub(crate) name: String,
    pub(crate) attrs: Vec<(String, String)>,
    pub(crate) text: String,
    pub(crate) children: Vec<Element>,
}

impl Element {
    /// The first child named `name`.
    pub(crate) fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }

    /// Every child named `name`.
    pub(crate) fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// Follow a path of child names.
    pub(crate) fn path(&self, names: &[&str]) -> Option<&Element> {
        names.iter().try_fold(self, |e, n| e.child(n))
    }

    /// Trimmed text of the child at `names`.
    pub(crate) fn text_at(&self, names: &[&str]) -> Option<&str> {
        self.path(names).map(|e| e.text.trim())
    }

    pub(crate) fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// Parse `text` into its root element.
pub(crate) fn parse(text: &str) -> Result<Element, String> {
    let mut p = Parser { s: text.as_bytes(), i: 0, nodes: 0 };
    p.skip_prolog()?;
    let root = p.element(0)?;
    p.skip_misc()?;
    if p.i < p.s.len() {
        return Err(p.err("content after the root element"));
    }
    Ok(root)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    nodes: usize,
}

impl Parser<'_> {
    fn err(&self, what: &str) -> String {
        format!("XML: {what} at byte {}", self.i)
    }

    fn starts(&self, pat: &str) -> bool {
        self.s.get(self.i..).is_some_and(|r| r.starts_with(pat.as_bytes()))
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            self.i += 1;
        }
    }

    /// Move past the next `pat`, or fail.
    fn skip_past(&mut self, pat: &str) -> Result<(), String> {
        let rest = self.s.get(self.i..).unwrap_or(&[]);
        match rest.windows(pat.len()).position(|w| w == pat.as_bytes()) {
            Some(at) => {
                self.i += at + pat.len();
                Ok(())
            }
            None => Err(self.err(&format!("missing `{pat}`"))),
        }
    }

    fn skip_misc(&mut self) -> Result<(), String> {
        loop {
            self.skip_ws();
            if self.starts("<?") {
                self.skip_past("?>")?;
            } else if self.starts("<!--") {
                self.skip_past("-->")?;
            } else if self.starts("<!DOCTYPE") {
                self.skip_past(">")?;
            } else {
                return Ok(());
            }
        }
    }

    fn skip_prolog(&mut self) -> Result<(), String> {
        if self.starts("\u{FEFF}") {
            self.i += 3;
        }
        self.skip_misc()
    }

    fn name(&mut self) -> Result<String, String> {
        let start = self.i;
        while self.peek().is_some_and(|c| !c.is_ascii_whitespace() && !matches!(c, b'>' | b'/' | b'=' | b'<' | b'"' | b'\'')) {
            self.i += 1;
        }
        if self.i == start {
            return Err(self.err("expected a name"));
        }
        let raw = self.s.get(start..self.i).unwrap_or(&[]);
        Ok(String::from_utf8_lossy(raw).into_owned())
    }

    fn element(&mut self, depth: usize) -> Result<Element, String> {
        if depth >= MAX_DEPTH {
            return Err(self.err("nested too deep"));
        }
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(self.err("too many elements"));
        }
        if self.peek() != Some(b'<') {
            return Err(self.err("expected `<`"));
        }
        self.i += 1;
        let mut el = Element { name: self.name()?, ..Element::default() };
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'/') => {
                    if !self.starts("/>") {
                        return Err(self.err("expected `/>`"));
                    }
                    self.i += 2;
                    return Ok(el);
                }
                Some(b'>') => {
                    self.i += 1;
                    break;
                }
                Some(_) => {
                    let key = self.name()?;
                    self.skip_ws();
                    if self.peek() != Some(b'=') {
                        return Err(self.err("expected `=`"));
                    }
                    self.i += 1;
                    self.skip_ws();
                    let q = self.peek().filter(|c| matches!(c, b'"' | b'\'')).ok_or_else(|| self.err("expected a quote"))?;
                    self.i += 1;
                    let start = self.i;
                    while self.peek().is_some_and(|c| c != q) {
                        self.i += 1;
                    }
                    if self.peek().is_none() {
                        return Err(self.err("unterminated attribute"));
                    }
                    let raw = String::from_utf8_lossy(self.s.get(start..self.i).unwrap_or(&[])).into_owned();
                    self.i += 1;
                    el.attrs.push((key, decode(&raw)));
                }
                None => return Err(self.err("unterminated tag")),
            }
        }
        // Content.
        loop {
            if self.starts("</") {
                self.i += 2;
                let end = self.name()?;
                if end != el.name {
                    return Err(self.err(&format!("`</{end}>` closes `<{}>`", el.name)));
                }
                self.skip_ws();
                if self.peek() != Some(b'>') {
                    return Err(self.err("expected `>`"));
                }
                self.i += 1;
                return Ok(el);
            } else if self.starts("<![CDATA[") {
                self.i += 9;
                let start = self.i;
                self.skip_past("]]>")?;
                let raw = self.s.get(start..self.i.saturating_sub(3)).unwrap_or(&[]);
                el.text.push_str(&String::from_utf8_lossy(raw));
            } else if self.starts("<!--") {
                self.skip_past("-->")?;
            } else if self.starts("<?") {
                self.skip_past("?>")?;
            } else if self.peek() == Some(b'<') {
                let child = self.element(depth + 1)?;
                el.children.push(child);
            } else if self.peek().is_none() {
                return Err(self.err(&format!("`<{}>` is not closed", el.name)));
            } else {
                let start = self.i;
                while self.peek().is_some_and(|c| c != b'<') {
                    self.i += 1;
                }
                let raw = String::from_utf8_lossy(self.s.get(start..self.i).unwrap_or(&[])).into_owned();
                el.text.push_str(&decode(&raw));
            }
        }
    }
}

/// Decode entity and character references; an unknown one is kept as written.
fn decode(raw: &str) -> String {
    if !raw.contains('&') {
        return raw.to_string();
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(at) = rest.find('&') {
        out.push_str(rest.get(..at).unwrap_or(""));
        let tail = rest.get(at..).unwrap_or("");
        let Some(semi) = tail.find(';').filter(|s| *s <= 12) else {
            out.push('&');
            rest = tail.get(1..).unwrap_or("");
            continue;
        };
        let ent = tail.get(1..semi).unwrap_or("");
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .or_else(|| ent.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse::<u32>().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = tail.get(semi + 1..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = tail.get(1..).unwrap_or("");
            }
        }
    }
    out.push_str(rest);
    out
}

/// Escape text for an element or a double-quoted attribute, dropping characters XML cannot hold.
pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}' => out.push(c),
            _ => {}
        }
    }
    out
}

/// Text as a CDATA section (a `]]>` inside it is split across two sections).
pub(crate) fn cdata(s: &str) -> String {
    format!("<![CDATA[{}]]>", escape_cdata(s))
}

fn escape_cdata(s: &str) -> String {
    let clean: String = s
        .chars()
        .filter(|c| matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}'))
        .collect();
    clean.replace("]]>", "]]]]><![CDATA[>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elements_attributes_text_and_cdata() {
        let doc = "\u{FEFF}<?xml version=\"1.0\"?>\n<!-- c --><A x=\"1 &amp; 2\" y='q'><B>t&lt;x&#65;&#x42;</B><C/><!-- d --><D><![CDATA[a]]b<c>]]></D></A>\n";
        let a = parse(doc).unwrap();
        assert_eq!(a.name, "A");
        assert_eq!(a.attr("x"), Some("1 & 2"));
        assert_eq!(a.attr("y"), Some("q"));
        assert_eq!(a.text_at(&["B"]), Some("t<xAB"));
        assert!(a.child("C").is_some());
        assert_eq!(a.text_at(&["D"]), Some("a]]b<c>"));
        assert_eq!(decode("&bogus; &; &#xZZ; &"), "&bogus; &; &#xZZ; &");
    }

    #[test]
    fn malformed_and_hostile_documents_are_errors() {
        for bad in ["", "<A>", "<A></B>", "<A x=1/>", "<A x=\"1/>", "<A><![CDATA[x</A>", "<A/><B/>", "<", "<A><!-- x</A>"] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        let deep = "<a>".repeat(200) + &"</a>".repeat(200);
        assert!(parse(&deep).is_err());
    }

    #[test]
    fn escaping_round_trips() {
        let s = "a<b>&\"c\" ]]> \u{1}";
        let doc = format!("<A v=\"{}\">{}{}</A>", escape(s), escape(s), cdata(s));
        let a = parse(&doc).unwrap();
        assert_eq!(a.attr("v"), Some("a<b>&\"c\" ]]> "));
        assert_eq!(a.text, "a<b>&\"c\" ]]> a<b>&\"c\" ]]> ");
    }
}
