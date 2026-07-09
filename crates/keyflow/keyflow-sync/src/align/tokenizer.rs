//! Vocabulary + transcript tokenization for CTC forced alignment.
//!
//! A CTC acoustic model emits over a fixed label set (characters plus a blank
//! and a word separator). To align known lyrics we must spell them into that
//! same label set. This module owns the vocab, the text normalization, and the
//! bookkeeping that lets us regroup per-token alignment spans back into words.

use std::collections::HashMap;

use crate::error::{Result, SyncError};

/// The label set of a CTC model. `labels[i]` is the surface form of token id
/// `i`. By convention id 0 is the blank. Some models (wav2vec2) have an explicit
/// word-separator label (`"|"`); others (MMS_FA) have none and tokenize words as
/// adjacent characters, tracking word boundaries externally — hence `separator`
/// is optional.
#[derive(Debug, Clone)]
pub struct Vocab {
    labels: Vec<String>,
    index: HashMap<String, u32>,
    pub blank: u32,
    pub separator: Option<u32>,
    /// Lowercase the transcript before tokenizing (MMS_FA). Set false for the
    /// uppercase wav2vec2 960h label set.
    pub lowercase: bool,
}

impl Vocab {
    /// Build from an ordered label list. `blank_label` names the blank entry;
    /// `sep_label` is the word separator, or `None` for models without one.
    pub fn new(
        labels: Vec<String>,
        blank_label: &str,
        sep_label: Option<&str>,
        lowercase: bool,
    ) -> Result<Self> {
        let index: HashMap<String, u32> = labels
            .iter()
            .enumerate()
            .map(|(i, l)| (l.clone(), i as u32))
            .collect();
        let blank = *index.get(blank_label).ok_or_else(|| {
            SyncError::Tokenize(format!("blank label {blank_label:?} not in vocab"))
        })?;
        let separator = match sep_label {
            Some(s) => Some(
                *index
                    .get(s)
                    .ok_or_else(|| SyncError::Tokenize(format!("separator {s:?} not in vocab")))?,
            ),
            None => None,
        };
        Ok(Self {
            labels,
            index,
            blank,
            separator,
            lowercase,
        })
    }

    /// Build from a HuggingFace-style `{token: id}` vocab map (as shipped in
    /// `vocab.json`). The blank is the token at id 0; the separator is `"|"` if
    /// present, else `None` (MMS_FA style). Case is auto-detected from the label
    /// set — uppercase letters (wav2vec2 960h) ⇒ uppercase the transcript;
    /// lowercase letters (MMS) ⇒ lowercase it. Works for both model families.
    pub fn from_vocab_map_json(json: &str) -> Result<Self> {
        let map: HashMap<String, u32> = facet_json::from_str(json)
            .map_err(|e| SyncError::Tokenize(format!("parse vocab map: {e}")))?;
        if map.is_empty() {
            return Err(SyncError::Tokenize("empty vocab map".into()));
        }
        let max_id = *map.values().max().unwrap() as usize;
        let mut labels = vec![String::new(); max_id + 1];
        for (tok, &id) in &map {
            labels[id as usize] = tok.clone();
        }
        let blank = labels
            .first()
            .cloned()
            .ok_or_else(|| SyncError::Tokenize("vocab map has no id 0".into()))?;
        let sep = if map.contains_key("|") {
            Some("|")
        } else {
            None
        };
        // If the alphabet is uppercase, don't lowercase the transcript.
        let has_upper = map
            .keys()
            .any(|k| k.len() == 1 && k.chars().all(|c| c.is_ascii_uppercase()));
        Self::new(labels, &blank, sep, !has_upper)
    }

    pub fn len(&self) -> usize {
        self.labels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }

    fn id_of(&self, ch: &str) -> Option<u32> {
        self.index.get(ch).copied()
    }

    /// Surface form of a token id (for decoding emissions back to text).
    pub fn label(&self, id: u32) -> &str {
        self.labels
            .get(id as usize)
            .map(|s| s.as_str())
            .unwrap_or("")
    }

    /// The default English wav2vec2 (960h) CTC label set: blank `<pad>` at id 0,
    /// word separator `|`, uppercase letters and apostrophe. Use this when no
    /// model-specific `*.vocab.json` is supplied.
    pub fn wav2vec2_en_960h() -> Self {
        let mut labels = vec![
            "<pad>".to_string(),
            "<s>".to_string(),
            "</s>".to_string(),
            "<unk>".to_string(),
            "|".to_string(),
        ];
        for c in 'A'..='Z' {
            labels.push(c.to_string());
        }
        labels.push("'".to_string());
        // Uppercase label set, so don't lowercase the transcript.
        Self::new(labels, "<pad>", Some("|"), false).expect("static wav2vec2 vocab is valid")
    }

    /// Build from a JSON array of labels (ordered by id). The blank is index 0;
    /// the separator is `|` if present, else none. For HF `{token: id}` maps use
    /// [`Vocab::from_vocab_map_json`] instead.
    pub fn from_labels_json(json: &str, lowercase: bool) -> Result<Self> {
        let labels: Vec<String> = facet_json::from_str(json)
            .map_err(|e| SyncError::Tokenize(format!("parse vocab json: {e}")))?;
        if labels.is_empty() {
            return Err(SyncError::Tokenize("empty vocab json".into()));
        }
        let blank = labels[0].clone();
        let sep = if labels.iter().any(|l| l == "|") {
            Some("|")
        } else {
            None
        };
        Self::new(labels.clone(), &blank, sep, lowercase)
    }
}

/// A transcript spelled into token ids, with a parallel map of which word each
/// token belongs to (separators map to `None`) so spans can be regrouped.
#[derive(Debug, Clone)]
pub struct Tokenized {
    pub tokens: Vec<u32>,
    pub token_word: Vec<Option<usize>>,
    pub words: Vec<String>,
}

/// Normalize then tokenize free text against `vocab`.
///
/// Normalization: optional lowercasing, drop characters with no vocab entry
/// (apostrophes, punctuation usually have none), collapse runs of whitespace to
/// a single separator. Characters the model *does* know but that aren't letters
/// (e.g. `'`) survive. Unknown letters are dropped with a trace, rather than
/// failing the whole song — a single odd glyph shouldn't sink alignment.
pub fn tokenize(text: &str, vocab: &Vocab) -> Result<Tokenized> {
    let mut tokens = Vec::new();
    let mut token_word = Vec::new();
    let mut words = Vec::new();

    let mut cur_word = String::new();
    let mut cur_word_idx: Option<usize> = None;
    let mut pending_sep = false;

    let flush_word = |words: &mut Vec<String>, cur: &mut String| {
        if !cur.is_empty() {
            words.push(std::mem::take(cur));
        }
    };

    for raw in text.chars() {
        let c = if vocab.lowercase {
            raw.to_ascii_lowercase()
        } else {
            raw.to_ascii_uppercase()
        };

        if c.is_whitespace() {
            if cur_word_idx.is_some() {
                pending_sep = true;
            }
            flush_word(&mut words, &mut cur_word);
            cur_word_idx = None;
            continue;
        }

        let s = c.to_string();
        let Some(id) = vocab.id_of(&s) else {
            tracing::trace!("dropping char {c:?} (not in vocab)");
            continue;
        };

        // Emit a separator between words, lazily, only once we know a real
        // letter follows it — and only for models that have a separator token.
        // MMS_FA has none; words are simply adjacent characters there.
        if pending_sep {
            if let Some(sep) = vocab.separator {
                tokens.push(sep);
                token_word.push(None);
            }
            pending_sep = false;
        }
        if cur_word_idx.is_none() {
            cur_word_idx = Some(words.len());
        }
        tokens.push(id);
        token_word.push(cur_word_idx);
        cur_word.push(c);
    }
    flush_word(&mut words, &mut cur_word);

    if tokens.is_empty() {
        return Err(SyncError::Tokenize(
            "transcript produced no tokens (no vocab-covered characters)".into(),
        ));
    }
    Ok(Tokenized {
        tokens,
        token_word,
        words,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ascii_vocab() -> Vocab {
        // 0 blank, 1 '|', then a..z
        let mut labels = vec!["<blank>".to_string(), "|".to_string()];
        for c in 'a'..='z' {
            labels.push(c.to_string());
        }
        Vocab::new(labels, "<blank>", Some("|"), true).unwrap()
    }

    #[test]
    fn tokenizes_words_with_separators() {
        let v = ascii_vocab();
        let t = tokenize("Hi there", &v).unwrap();
        assert_eq!(t.words, vec!["hi", "there"]);
        // one separator between the two words, none leading/trailing.
        let sep = v.separator.unwrap();
        let seps = t.tokens.iter().filter(|&&x| x == sep).count();
        assert_eq!(seps, 1);
        // first two tokens belong to word 0, last to word 1.
        assert_eq!(t.token_word[0], Some(0));
        assert_eq!(*t.token_word.last().unwrap(), Some(1));
    }

    #[test]
    fn drops_unknown_chars_but_keeps_word() {
        let v = ascii_vocab();
        let t = tokenize("don't", &v).unwrap();
        // apostrophe dropped, letters kept as one word.
        assert_eq!(t.words, vec!["dont"]);
    }

    #[test]
    fn empty_after_normalization_errors() {
        let v = ascii_vocab();
        assert!(tokenize("123 !!!", &v).is_err());
    }
}
