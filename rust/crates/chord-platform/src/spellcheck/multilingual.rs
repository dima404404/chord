//! Combines dictionaries for the active languages and shares learned words.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::path::Path;

use super::dictionary::resolve_dictionary_paths_for_tag;
use super::{
    MAX_SUGGESTIONS, SYSTEM_HUNSPELL_DIR, SpellChecker, SpellError, SpellLocale, Token,
    heuristic_suggestions, is_ignored_special_token, strip_outer_punctuation, tokenize,
};

/// Multi-language native spellchecker aggregating multiple `SpellChecker` instances
/// with shared custom dictionary (`learned_words` / `removed_words`) and enable toggle.
pub struct MultiLanguageSpellChecker {
    checkers: Vec<(String, SpellChecker)>,
    learned_words: BTreeSet<String>,
    removed_words: BTreeSet<String>,
    enabled: bool,
}

impl Default for MultiLanguageSpellChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiLanguageSpellChecker {
    #[must_use]
    pub fn new() -> Self {
        Self {
            checkers: Vec::new(),
            learned_words: BTreeSet::new(),
            removed_words: BTreeSet::new(),
            enabled: true,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub fn active_languages(&self) -> Vec<String> {
        self.checkers.iter().map(|(lang, _)| lang.clone()).collect()
    }

    #[must_use]
    pub const fn learned_words(&self) -> &BTreeSet<String> {
        &self.learned_words
    }

    pub fn set_learned_words(&mut self, words: impl IntoIterator<Item = String>) {
        self.learned_words = words
            .into_iter()
            .map(|w| w.trim().to_lowercase())
            .filter(|w| !w.is_empty() && !w.contains('\0'))
            .collect();
        for word in &self.learned_words {
            self.removed_words.remove(word);
            for (_, checker) in &mut self.checkers {
                let _ = checker.add(word);
            }
        }
    }

    pub fn set_languages(
        &mut self,
        locales: &[&str],
        search_dirs: &[&Path],
    ) -> Result<Vec<String>, SpellError> {
        let mut next = Vec::new();
        let mut loaded_tags = Vec::new();
        for raw in locales {
            let tag = SpellLocale::parse(raw)
                .map_or_else(|| raw.trim().replace('-', "_"), |l| l.as_tag().to_string());
            if tag.is_empty() || loaded_tags.contains(&tag) {
                continue;
            }
            let (aff, dic) =
                resolve_dictionary_paths_for_tag(search_dirs, &tag).ok_or_else(|| {
                    SpellError::Missing {
                        path: format!("{SYSTEM_HUNSPELL_DIR}/{tag}.dic"),
                        reason: "словарь локали не найден".to_owned(),
                    }
                })?;
            let mut checker = SpellChecker::new(&aff, &dic)?;
            for word in &self.learned_words {
                let _ = checker.add(word);
            }
            for word in &self.removed_words {
                let _ = checker.remove(word);
            }
            loaded_tags.push(tag.clone());
            next.push((tag, checker));
        }
        self.checkers = next;
        Ok(loaded_tags)
    }

    pub fn check_word(&mut self, raw_word: &str) -> bool {
        if !self.enabled {
            return true;
        }
        let trimmed = raw_word.trim();
        if trimmed.is_empty() || is_ignored_special_token(trimmed) {
            return true;
        }
        let Some(cleaned) = strip_outer_punctuation(trimmed) else {
            return true;
        };
        let lower = cleaned.to_lowercase();
        if self.removed_words.contains(&lower) {
            return false;
        }
        if self.learned_words.contains(&lower) {
            return true;
        }
        if self.checkers.is_empty() {
            return true;
        }
        self.checkers
            .iter_mut()
            .any(|(_, checker)| checker.check_word(cleaned))
    }

    pub fn suggest(&mut self, word: &str) -> Vec<String> {
        let cleaned = strip_outer_punctuation(word.trim()).unwrap_or("");
        if cleaned.is_empty() || cleaned.contains('\0') {
            return Vec::new();
        }
        let mut combined = Vec::new();
        for (_, checker) in &mut self.checkers {
            for item in checker.suggest(cleaned) {
                if self.removed_words.contains(&item.to_lowercase()) {
                    continue;
                }
                if combined.len() < MAX_SUGGESTIONS && !combined.contains(&item) {
                    combined.push(item);
                }
            }
        }
        if combined.len() < MAX_SUGGESTIONS {
            let empty_lexicon = BTreeSet::new();
            for item in heuristic_suggestions(
                cleaned,
                &empty_lexicon,
                &self.learned_words,
                &self.removed_words,
            ) {
                if combined.len() < MAX_SUGGESTIONS && !combined.contains(&item) {
                    combined.push(item);
                }
            }
        }
        combined
    }

    pub fn add_word(&mut self, word: &str) -> bool {
        if word.is_empty() || word.contains('\0') {
            return false;
        }
        let lower = word.to_lowercase();
        self.removed_words.remove(&lower);
        self.learned_words.insert(lower);
        let mut ok = true;
        for (_, checker) in &mut self.checkers {
            if !checker.add(word) {
                ok = false;
            }
        }
        ok
    }

    pub fn remove_word(&mut self, word: &str) -> bool {
        if word.is_empty() || word.contains('\0') {
            return false;
        }
        let lower = word.to_lowercase();
        self.learned_words.remove(&lower);
        self.removed_words.insert(lower);
        let mut ok = true;
        for (_, checker) in &mut self.checkers {
            if !checker.remove(word) {
                ok = false;
            }
        }
        ok
    }

    pub fn misspelled_tokens(&mut self, text: &str) -> Vec<(Token, Vec<String>)> {
        if !self.enabled {
            return Vec::new();
        }
        let mut result = Vec::new();
        for token in tokenize(text) {
            if !self.check_word(&token.text) {
                let suggestions = self.suggest(&token.text);
                result.push((token, suggestions));
            }
        }
        result
    }
}
