//! Track tagging: categories on tracks (see [`crate::Track::tags`]), what the smart scan learned
//! from the user's fixes, and whether tags drive grouping.

use std::collections::BTreeMap;

/// Most tags on one track.
pub const MAX_TAGS: usize = 16;
/// Most characters in one tag.
pub const MAX_TAG_CHARS: usize = 32;
/// Most entries in each table of [`Tagging`]; a hostile file cannot make it grow past this.
pub const MAX_LEARNED: usize = 2048;

/// Session-wide tagging state.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Tagging {
    /// Tags drive grouping: tagged tracks collect into folders and busses, and move when retagged.
    #[serde(default)]
    pub group: bool,
    /// Tags the user confirmed for a track name (its words, lower case, numbers left out),
    /// used as is on the next scan.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub names: BTreeMap<String, Vec<String>>,
    /// Tags the user added for a word the scan does not know, such as a singer's name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub words: BTreeMap<String, Vec<String>>,
}

impl Tagging {
    pub fn is_empty(&self) -> bool {
        *self == Tagging::default()
    }
}

impl crate::Session {
    /// Repair tags after loading untrusted data: cleaned, capped tags and learned tables.
    pub(crate) fn sanitize_tags(&mut self) {
        for t in &mut self.tracks {
            if !t.tags.is_empty() {
                t.tags = clean_tags(t.tags.iter().map(String::as_str));
            }
            if let Some(g) = &mut t.tag_group {
                *g = g.chars().take(MAX_TAG_CHARS * 4).collect();
            }
        }
        for table in [&mut self.tagging.names, &mut self.tagging.words] {
            let keep: BTreeMap<String, Vec<String>> = std::mem::take(table)
                .into_iter()
                .filter(|(k, _)| k.chars().count() <= 256)
                .map(|(k, v)| (k, clean_tags(v.iter().map(String::as_str))))
                .take(MAX_LEARNED)
                .collect();
            *table = keep;
        }
    }
}

/// A tag as stored: trimmed, inner spaces collapsed, at most [`MAX_TAG_CHARS`] characters, no
/// control characters or commas (a comma separates tags where they are typed). `None` if nothing
/// is left.
pub fn clean_tag(raw: &str) -> Option<String> {
    let words: Vec<&str> = raw.split(|c: char| c.is_whitespace() || c.is_control() || c == ',').filter(|w| !w.is_empty()).collect();
    let tag: String = words.join(" ").chars().take(MAX_TAG_CHARS).collect();
    let tag = tag.trim_end().to_string();
    (!tag.is_empty()).then_some(tag)
}

/// Clean `raw` tags, dropping empty ones and repeats (ignoring case), keeping the first
/// [`MAX_TAGS`].
pub fn clean_tags<'a>(raw: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in raw.into_iter().filter_map(clean_tag) {
        if out.len() >= MAX_TAGS {
            break;
        }
        if !out.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
            out.push(tag);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_cleaned_and_capped() {
        assert_eq!(clean_tag("  Lead   Vox \n"), Some("Lead Vox".into()));
        assert_eq!(clean_tag(" , \t"), None);
        assert_eq!(clean_tag(&"x".repeat(100)).map(|t| t.len()), Some(MAX_TAG_CHARS));
        assert_eq!(clean_tags(["Vocals", "vocals", "", "Dubs"]), ["Vocals", "Dubs"]);
        let many: Vec<String> = (0..40).map(|i| format!("t{i}")).collect();
        assert_eq!(clean_tags(many.iter().map(String::as_str)).len(), MAX_TAGS);
    }

    #[test]
    fn hostile_tags_in_a_file_are_repaired() {
        let mut s = crate::Session::default();
        let t = s.add_track(crate::TrackKind::Audio, crate::ChannelFormat::Mono, Some("Vox"));
        if let Some(tr) = s.track_mut(t) {
            tr.tags = (0..100).map(|i| format!(" t{i}\u{0} ")).collect();
        }
        for i in 0..5000 {
            s.tagging.words.insert(format!("w{i}"), vec!["Vocals".into(), "vocals".into()]);
        }
        s.sanitize_tags();
        let tr = s.track(t).unwrap();
        assert_eq!(tr.tags.len(), MAX_TAGS);
        assert_eq!(tr.tags[0], "t0");
        assert_eq!(s.tagging.words.len(), MAX_LEARNED);
        assert!(s.tagging.words.values().all(|v| v == &["Vocals"]));
    }
}
