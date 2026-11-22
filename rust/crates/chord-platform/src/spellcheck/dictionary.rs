//! Finds and validates Hunspell dictionary pairs.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::{FALLBACK_MYSPELL_DIR, SYSTEM_HUNSPELL_DIR, SpellError, SpellLocale, inv_dict};

#[must_use]
pub fn resolve_system_dictionary_paths(
    search_dirs: &[&Path],
    locale: &str,
) -> Option<(PathBuf, PathBuf)> {
    let tag = SpellLocale::parse(locale).map_or(locale, |l| l.as_tag());
    let sys = [
        Path::new(SYSTEM_HUNSPELL_DIR),
        Path::new(FALLBACK_MYSPELL_DIR),
    ];
    for dir in search_dirs.iter().copied().chain(sys) {
        let (aff, dic) = (
            dir.join(format!("{tag}.aff")),
            dir.join(format!("{tag}.dic")),
        );
        if aff.is_file() && dic.is_file() {
            return Some((aff, dic));
        }
    }
    search_dirs.iter().find_map(|dir| {
        let (aff, dic) = (dir.join("test.aff"), dir.join("test.dic"));
        (aff.is_file() && dic.is_file()).then_some((aff, dic))
    })
}

/// Scans `search_dirs` and system hunspell/myspell directories for available `<locale>.aff` + `<locale>.dic` pairs.
#[must_use]
pub fn list_available_dictionaries(search_dirs: &[&Path]) -> Vec<String> {
    let sys = [
        Path::new(SYSTEM_HUNSPELL_DIR),
        Path::new(FALLBACK_MYSPELL_DIR),
    ];
    let mut found = BTreeSet::new();
    for dir in search_dirs.iter().copied().chain(sys) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("dic") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if dir.join(format!("{stem}.aff")).is_file() {
                found.insert(stem.to_string());
            }
        }
    }
    found.into_iter().collect()
}

pub(super) fn resolve_dictionary_paths_for_tag(
    search_dirs: &[&Path],
    tag: &str,
) -> Option<(PathBuf, PathBuf)> {
    let normalized = tag.trim().replace('-', "_");
    let sys = [
        Path::new(SYSTEM_HUNSPELL_DIR),
        Path::new(FALLBACK_MYSPELL_DIR),
    ];
    for dir in search_dirs.iter().copied().chain(sys) {
        let (aff, dic) = (
            dir.join(format!("{normalized}.aff")),
            dir.join(format!("{normalized}.dic")),
        );
        if aff.is_file() && dic.is_file() {
            return Some((aff, dic));
        }
    }
    resolve_system_dictionary_paths(search_dirs, &normalized)
}

pub(super) fn path_text(path: &Path) -> Result<&str, SpellError> {
    let text = path.to_str().ok_or_else(|| SpellError::InvalidPath {
        path: path.display().to_string(),
    })?;
    if text.contains('\0') {
        return Err(SpellError::Nul {
            path: path.display().to_string(),
        });
    }
    Ok(text)
}

fn read_non_empty_file(path: &Path, empty_reason: &str) -> Result<String, SpellError> {
    let metadata = std::fs::metadata(path).map_err(|e| SpellError::Missing {
        path: path.display().to_string(),
        reason: e.to_string(),
    })?;
    if !metadata.is_file() {
        return Err(SpellError::Missing {
            path: path.display().to_string(),
            reason: "не обычный файл".to_owned(),
        });
    }
    if metadata.len() == 0 {
        return Err(inv_dict(path, empty_reason));
    }
    std::fs::read_to_string(path).map_err(|e| inv_dict(path, format!("ошибка UTF-8: {e}")))
}

pub(super) fn validate_aff(path: &Path) -> Result<(), SpellError> {
    let text = read_non_empty_file(path, "aff-файл пуст")?;
    if !text.lines().any(|l| {
        l.trim_start_matches('\u{feff}')
            .trim_start()
            .starts_with("SET ")
    }) {
        return Err(inv_dict(path, "aff без директивы SET"));
    }
    Ok(())
}

pub(super) fn validate_dic(path: &Path) -> Result<BTreeSet<String>, SpellError> {
    let text = read_non_empty_file(path, "dic-файл пуст")?;
    let mut lines = text.lines();
    let header = lines
        .next()
        .unwrap_or("")
        .trim_start_matches('\u{feff}')
        .trim();
    let count = header
        .parse::<usize>()
        .ok()
        .filter(|c| *c > 0)
        .ok_or_else(|| inv_dict(path, format!("некорректный заголовок dic: {header:?}")))?;
    let entries: Vec<&str> = lines.map(str::trim).filter(|l| !l.is_empty()).collect();
    if entries.len() != count {
        return Err(inv_dict(path, format!("dic: {count} != {}", entries.len())));
    }
    let mut lexicon = BTreeSet::new();
    for entry in entries {
        let word = entry.split(['/', ' ', '\t']).next().unwrap_or_default();
        if word.is_empty()
            || !word
                .chars()
                .all(|c| c.is_alphabetic() || matches!(c, '\'' | '-'))
        {
            return Err(inv_dict(
                path,
                format!("dic: недопустимая запись {entry:?}"),
            ));
        }
        lexicon.insert(word.to_lowercase());
    }
    Ok(lexicon)
}
