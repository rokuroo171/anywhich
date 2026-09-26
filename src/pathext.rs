// The rules are called from cfg(windows) code only, so on other platforms
// they are intentionally unused; keeping them compiled and tested everywhere
// is the point of the module.
#![cfg_attr(not(windows), allow(dead_code))]

/// Windows PATHEXT lookup rules, as pure functions.
///
/// Compiled on every platform so the rules are testable from Unix, where CI
/// runs. The `cfg(windows)` call sites live in pathwalk.rs; nothing here
/// touches the filesystem.

/// The extensions Windows uses when PATHEXT is not set, in the classic
/// order. Windows accepts the entry with or without the leading dot;
/// both forms are normalized here.
pub const DEFAULT_PATHEXT: [&str; 4] = [".COM", ".EXE", ".BAT", ".CMD"];

/// (extension, is_from_default_set) pairs in PATHEXT order, as typed:
/// case-insensitive parsing, but the original case is kept because
/// `candidate_names` must try the file exactly as PATHEXT spells it.
pub fn parse(raw: Option<&str>) -> Vec<(String, bool)> {
    let raw = match raw {
        Some(r) if !r.trim().is_empty() => r,
        _ => {
            return DEFAULT_PATHEXT
                .iter()
                .map(|e| (e.to_string(), true))
                .collect()
        }
    };
    raw.split(';')
        .filter_map(|entry| {
            let entry = entry.trim();
            if entry.is_empty() {
                return None;
            }
            let dot = if entry.starts_with('.') {
                entry.to_string()
            } else {
                format!(".{entry}")
            };
            let is_default = DEFAULT_PATHEXT
                .iter()
                .any(|d| d.eq_ignore_ascii_case(&dot));
            Some((dot, is_default))
        })
        .collect()
}

/// The name variants to look for, in the order Windows would try them:
/// the typed name first, then the typed name plus each PATHEXT extension,
/// unless the typed name already ends in one of the listed extensions
/// (case-insensitive). A typed name with a non-PATHEXT extension ("tool.sh")
/// is a plain name as far as PATHEXT lookup is concerned, so the extensions
/// are still appended.
pub fn candidate_names(name: &str, pathext: &[(String, bool)]) -> Vec<String> {
    let lower = name.to_ascii_lowercase();
    let has_listed_ext = pathext
        .iter()
        .any(|(ext, _)| lower.ends_with(&ext.to_ascii_lowercase()));
    if has_listed_ext {
        return vec![name.to_string()];
    }
    let mut names = vec![name.to_string()];
    for (ext, _) in pathext {
        names.push(format!("{name}{ext}"));
    }
    names
}

/// Does `file_name` end in one of the PATHEXT extensions? On Windows this is
/// what "executable" means for PATH lookup; files with other extensions are
/// data, not commands, however their permissions read.
pub fn is_executable_extension(file_name: &str, pathext: &[(String, bool)]) -> bool {
    let lower = file_name.to_ascii_lowercase();
    pathext
        .iter()
        .any(|(ext, _)| lower.ends_with(&ext.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exts(raw: Option<&str>) -> Vec<(String, bool)> {
        parse(raw)
    }

    #[test]
    fn unset_or_blank_pathext_gives_the_default_set() {
        assert_eq!(exts(None).len(), 4);
        assert_eq!(exts(None)[1], (".EXE".to_string(), true));
        assert_eq!(exts(Some("  ")).len(), 4);
    }

    #[test]
    fn entries_are_normalized_and_kept_in_given_order() {
        let e = exts(Some(".ZIP;.bat;COM"));
        assert_eq!(
            e,
            vec![
                (".ZIP".to_string(), false),
                (".bat".to_string(), true),
                (".COM".to_string(), true),
            ]
        );
    }

    #[test]
    fn empty_entries_are_dropped() {
        assert_eq!(exts(Some(".EXE;;.BAT")).len(), 2);
    }

    #[test]
    fn bare_name_expands_per_pathext_order() {
        let e = exts(Some(".COM;.EXE;.BAT"));
        assert_eq!(
            candidate_names("python", &e),
            vec!["python", "python.COM", "python.EXE", "python.BAT"]
        );
    }

    #[test]
    fn typed_extension_is_tried_once_as_given() {
        let e = exts(Some(".EXE;.BAT"));
        assert_eq!(candidate_names("python.exe", &e), vec!["python.exe"]);
        assert_eq!(candidate_names("python.EXE", &e), vec!["python.EXE"]);
    }

    #[test]
    fn non_pathext_extension_still_expands() {
        let e = exts(Some(".EXE;.BAT"));
        assert_eq!(
            candidate_names("tool.sh", &e),
            vec!["tool.sh", "tool.sh.EXE", "tool.sh.BAT"]
        );
    }

    #[test]
    fn executable_extension_check_is_case_insensitive() {
        let e = exts(Some(".EXE;.BAT"));
        assert!(is_executable_extension("python.EXE", &e));
        assert!(is_executable_extension("python.exe", &e));
        assert!(is_executable_extension("run.BAT", &e));
        assert!(!is_executable_extension("notes.txt", &e));
        assert!(!is_executable_extension("data", &e));
    }
}
