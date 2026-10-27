//! Renaming the photos of one folder from a pattern, in the way of
//! exiftool's `-filename<${DateTimeOriginal}%-03c.%le` with `-d` for the date
//! format: the same keywords and copy numbers, every new name checked before
//! any file is touched, and all files renamed or none.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::format::{Item, StrftimeItems};

use crate::i18n::{tr, trf};

/// The keywords offered under the pattern, as they are typed into it.
pub const KEYWORDS: [&str; 7] = ["${DateTimeOriginal}", "${FileModifyDate}", "${Model}", "%f", "%-03c", "%le", "%e"];
/// The pattern and date format of the exiftool command most people start from.
pub const PATTERN: &str = "${DateTimeOriginal}%-03c.%le";
pub const DATE_FORMAT: &str = "%Y.%m.%d-%Hh%Mm%Ss";

/// Characters a file name can't have on Windows, and so on no system, since
/// folders move between them.
const NOT_IN_NAMES: &str = "\\/:*?\"<>|";
const TEMPORARY: &str = ".picasso-renaming";

/// One photo of the folder.
pub struct File {
    pub id: i64,
    pub name: String,
    /// Date taken and date modified, in seconds since 1970.
    pub taken_at: i64,
    pub modified_at: i64,
    pub camera: String,
}

/// What becomes of one file.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Rename(String),
    Same,
    /// It keeps its name, for this reason.
    Fails(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Case {
    Keep,
    Lower,
    Upper,
}

/// `%c`, the copy number: after `lead`, padded with zeros to `width`, and
/// left out for the first file of a name unless `always`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Copy {
    lead: &'static str,
    width: usize,
    always: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Text(String),
    Taken,
    Modified,
    Model,
    Stem(Case),
    Extension(Case),
    Copy(Copy),
}

/// The new name of each of `files`, which are in folder order, beside the
/// `others` names in the folder that aren't being renamed. A copy number goes
/// to a file only when its name is taken, and a file already named by the
/// pattern keeps its name, so renaming twice changes nothing.
pub fn plan(pattern: &str, date_format: &str, files: &[File], others: &[String]) -> Result<Vec<Outcome>, String> {
    let tokens = parse(pattern)?;
    let dated = tokens.iter().any(|token| matches!(token, Token::Taken | Token::Modified));
    if dated && StrftimeItems::new(date_format).any(|item| matches!(item, Item::Error)) {
        return Err(tr("The date format isn't valid").into());
    }
    let copies: Vec<Copy> = tokens.iter().filter_map(|token| if let Token::Copy(copy) = token { Some(*copy) } else { None }).collect();
    let first = copies.iter().any(|copy| copy.always).then_some(0);
    let limit = (files.len() + others.len() + 1) as u32;
    let mut taken: HashSet<String> = others.iter().map(|name| key(name)).collect();
    let mut outcomes: Vec<Option<Outcome>> = vec![None; files.len()];
    let mut parts = Vec::with_capacity(files.len());
    for (file, outcome) in files.iter().zip(&mut outcomes) {
        let expanded = expand(&tokens, file, date_format).and_then(|expanded| match problem(&join(&expanded, &copies, first), &file.name) {
            Some(problem) => Err(problem),
            None => Ok(expanded),
        });
        match expanded {
            Err(reason) => *outcome = Some(Outcome::Fails(reason)),
            Ok(expanded) => {
                if let Some(number) = own_number(&expanded, &copies, first, &file.name, limit) {
                    let name = join(&expanded, &copies, number);
                    taken.insert(key(&name));
                    *outcome = Some(if name == file.name { Outcome::Same } else { Outcome::Rename(name) });
                }
                parts.push(Some(expanded));
                continue;
            }
        }
        parts.push(None);
    }
    for (expanded, outcome) in parts.iter().zip(&mut outcomes) {
        let Some(expanded) = expanded.as_ref().filter(|_| outcome.is_none()) else { continue };
        let mut number = first;
        *outcome = Some(loop {
            let name = join(expanded, &copies, number);
            if taken.insert(key(&name)) {
                break Outcome::Rename(name);
            }
            if copies.is_empty() {
                break Outcome::Fails(tr("The name is taken by another file").into());
            }
            number = Some(number.map_or(1, |n| n + 1));
        });
    }
    Ok(outcomes.into_iter().map(|outcome| outcome.unwrap_or(Outcome::Same)).collect())
}

fn parse(pattern: &str) -> Result<Vec<Token>, String> {
    fn push(tokens: &mut Vec<Token>, text: &mut String, token: Token) {
        if !text.is_empty() {
            tokens.push(Token::Text(std::mem::take(text)));
        }
        tokens.push(token);
    }
    let unknown = |code: &str| trf("Unknown keyword: {}", &[&code]);
    let mut tokens = Vec::new();
    let mut text = String::new();
    let mut rest = pattern;
    while let Some(c) = rest.chars().next() {
        let after = &rest[c.len_utf8()..];
        match c {
            '$' if after.starts_with('$') => {
                text.push('$');
                rest = &after[1..];
            }
            '$' => {
                let (name, next) = match after.strip_prefix('{') {
                    Some(inner) => {
                        let end = inner.find('}').ok_or_else(|| unknown(rest))?;
                        (&inner[..end], &inner[end + 1..])
                    }
                    None => {
                        let end = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(after.len());
                        (&after[..end], &after[end..])
                    }
                };
                let token = match name.to_ascii_lowercase().as_str() {
                    "datetimeoriginal" => Token::Taken,
                    "filemodifydate" => Token::Modified,
                    "model" => Token::Model,
                    _ => return Err(unknown(&rest[..rest.len() - next.len()])),
                };
                push(&mut tokens, &mut text, token);
                rest = next;
            }
            '%' if after.starts_with('%') => {
                text.push('%');
                rest = &after[1..];
            }
            '%' => {
                let (case, body) = match after.chars().next() {
                    Some('l') => (Case::Lower, &after[1..]),
                    Some('u') => (Case::Upper, &after[1..]),
                    _ => (Case::Keep, after),
                };
                if let Some(next) = body.strip_prefix('f') {
                    push(&mut tokens, &mut text, Token::Stem(case));
                    rest = next;
                    continue;
                }
                if let Some(next) = body.strip_prefix('e') {
                    push(&mut tokens, &mut text, Token::Extension(case));
                    rest = next;
                    continue;
                }
                let (lead, body) = match after.chars().next() {
                    Some('-') => ("-", &after[1..]),
                    Some('+') => ("_", &after[1..]),
                    _ => ("", after),
                };
                let digits = body.find(|c: char| !c.is_ascii_digit()).unwrap_or(body.len());
                let width = body[..digits].parse::<usize>().unwrap_or(0).min(9);
                let body = &body[digits..];
                let (always, body) = match body.strip_prefix('.') {
                    Some(body) => (true, body),
                    None => (false, body),
                };
                let Some(next) = body.strip_prefix('c') else {
                    return Err(unknown(&rest[..rest.len() - body.len() + body.chars().next().map_or(0, char::len_utf8)]));
                };
                push(&mut tokens, &mut text, Token::Copy(Copy { lead, width, always }));
                rest = next;
            }
            _ => {
                text.push(c);
                rest = after;
            }
        }
    }
    if !text.is_empty() {
        tokens.push(Token::Text(text));
    }
    Ok(tokens)
}

/// The new name of `file`, in parts that the copy numbers go between.
fn expand(tokens: &[Token], file: &File, date_format: &str) -> Result<Vec<String>, String> {
    let path = Path::new(&file.name);
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let extension = path.extension().map(|ext| ext.to_string_lossy().into_owned()).unwrap_or_default();
    let mut parts = vec![String::new()];
    for token in tokens {
        let text = match token {
            Token::Text(text) => text.clone(),
            Token::Taken => local_time(file.taken_at, date_format),
            Token::Modified => local_time(file.modified_at, date_format),
            Token::Model if file.camera.trim().is_empty() => return Err(tr("The camera isn't known").into()),
            Token::Model => file.camera.trim().chars().map(|c| if c < ' ' || NOT_IN_NAMES.contains(c) { '_' } else { c }).collect(),
            Token::Stem(case) => cased(&stem, *case),
            Token::Extension(case) => cased(&extension, *case),
            Token::Copy(_) => {
                parts.push(String::new());
                continue;
            }
        };
        parts.last_mut().unwrap().push_str(&text);
    }
    Ok(parts)
}

/// `secs` on the computer's clock. Photo dates are kept as read in local
/// time, and video dates in UTC, so this gives both as the camera showed
/// them, as exiftool does with `-api QuickTimeUTC=1`.
fn local_time(secs: i64, format: &str) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|time| time.with_timezone(&chrono::Local).format(format).to_string())
        .unwrap_or_default()
}

fn cased(text: &str, case: Case) -> String {
    match case {
        Case::Keep => text.to_string(),
        Case::Lower => text.to_lowercase(),
        Case::Upper => text.to_uppercase(),
    }
}

fn join(parts: &[String], copies: &[Copy], number: Option<u32>) -> String {
    let mut name = parts[0].clone();
    for (copy, part) in copies.iter().zip(&parts[1..]) {
        if let Some(number) = number {
            name.push_str(&format!("{}{number:0width$}", copy.lead, width = copy.width));
        }
        name.push_str(part);
    }
    name
}

/// The copy number with which the pattern gives the file the name it has.
fn own_number(parts: &[String], copies: &[Copy], first: Option<u32>, current: &str, limit: u32) -> Option<Option<u32>> {
    let current = key(current);
    if !current.starts_with(&key(&parts[0])) || !current.ends_with(&key(parts.last()?)) {
        return None;
    }
    if copies.is_empty() {
        return (key(&parts[0]) == current).then_some(None);
    }
    std::iter::once(first)
        .chain((first.map_or(1, |n| n + 1)..=limit).map(Some))
        .find(|number| key(&join(parts, copies, *number)) == current)
}

/// Why `name` can't replace `original`, if it can't.
fn problem(name: &str, original: &str) -> Option<String> {
    let extension = |name: &str| Path::new(name).extension().map(|ext| ext.to_string_lossy().to_lowercase());
    if name.is_empty() || extension(name) != extension(original) || Path::new(name).file_stem().is_none() {
        return Some(tr("The new name must end with the file's extension").into());
    }
    if name.chars().any(|c| c < ' ' || NOT_IN_NAMES.contains(c)) || name.ends_with([' ', '.']) {
        return Some(tr("The name has characters that aren't allowed").into());
    }
    (name.len() > 255).then(|| tr("The name is too long").into())
}

/// A name as the file system compares it: Windows and macOS don't tell
/// upper from lower case.
fn key(name: &str) -> String {
    if cfg!(any(windows, target_os = "macos")) {
        name.to_lowercase()
    } else {
        name.to_string()
    }
}

/// Renames files in `dir` from the first name of each pair to the second,
/// all of them or none. Each file takes a temporary name first, so names can
/// pass from one file to another, and no file is ever renamed over another.
pub fn apply(dir: &Path, renames: &[(String, String)]) -> Result<(), String> {
    let mut temporary = Vec::with_capacity(renames.len());
    for (from, _) in renames {
        let mut temp = dir.join(format!("{from}{TEMPORARY}"));
        for n in 1.. {
            if temp.symlink_metadata().is_err() {
                break;
            }
            temp = dir.join(format!("{from}{TEMPORARY}{n}"));
        }
        if let Err(err) = move_file(&dir.join(from), &temp) {
            return Err(put_back(dir, &renames[..temporary.len()], &temporary, 0, format!("{from}: {err}")));
        }
        temporary.push(temp);
    }
    for (done, (_, to)) in renames.iter().enumerate() {
        if let Err(err) = move_file(&temporary[done], &dir.join(to)) {
            return Err(put_back(dir, renames, &temporary, done, format!("{to}: {err}")));
        }
    }
    Ok(())
}

/// Gives every file its first name back, when the first `done` have their
/// new names and the others their temporary ones. Returns `error`, with the
/// files that couldn't go back.
fn put_back(dir: &Path, renames: &[(String, String)], temporary: &[PathBuf], done: usize, error: String) -> String {
    let mut stuck = Vec::new();
    for ((_, to), temp) in renames.iter().zip(temporary).take(done) {
        if move_file(&dir.join(to), temp).is_err() {
            stuck.push(to.clone());
        }
    }
    for ((from, to), temp) in renames.iter().zip(temporary) {
        if !stuck.contains(to) && move_file(temp, &dir.join(from)).is_err() {
            stuck.push(temp.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default());
        }
    }
    if stuck.is_empty() {
        error
    } else {
        format!("{error} ({}: {})", tr("These files kept another name"), stuck.join(", "))
    }
}

/// The file beside the photos that lists what Picasso renamed in the folder,
/// so the names can be put back later, by Picasso or by hand. Scans skip it,
/// as they skip every name starting with a dot.
pub const LOG: &str = ".picasso-renames.json";
/// Renamings the log keeps; older ones fall off.
const LOG_KEEP: usize = 50;

/// One renaming: when, in local time, and each file's old and new name.
#[derive(Clone, Debug, PartialEq)]
pub struct Batch {
    pub at: String,
    pub files: Vec<(String, String)>,
}

/// The renamings listed in `dir`, oldest first. A folder without a log has
/// none; a log that can't be read is an error, so it isn't written over.
pub fn history(dir: &Path) -> Result<Vec<Batch>, String> {
    let text = match std::fs::read_to_string(dir.join(LOG)) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(format!("{LOG}: {err}")),
    };
    let doc: serde_json::Value = serde_json::from_str(&text).map_err(|err| format!("{LOG}: {err}"))?;
    let batches = doc.get("renamings").and_then(|list| list.as_array()).ok_or_else(|| format!("{LOG}: renamings"))?;
    Ok(batches
        .iter()
        .map(|batch| Batch {
            at: batch.get("at").and_then(|at| at.as_str()).unwrap_or_default().to_string(),
            files: batch
                .get("files")
                .and_then(|files| files.as_array())
                .into_iter()
                .flatten()
                .filter_map(|file| Some((file.get("from")?.as_str()?.to_string(), file.get("to")?.as_str()?.to_string())))
                .collect(),
        })
        .filter(|batch| !batch.files.is_empty())
        .collect())
}

/// Adds the renaming of `files`, as (old name, new name), to the log in `dir`.
pub fn record(dir: &Path, files: &[(String, String)]) -> Result<(), String> {
    let mut batches = history(dir)?;
    batches.push(Batch { at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(), files: files.to_vec() });
    let extra = batches.len().saturating_sub(LOG_KEEP);
    batches.drain(..extra);
    save_history(dir, &batches)
}

/// Takes the newest renaming off the log in `dir`, once it has been undone.
pub fn forget_last(dir: &Path) -> Result<(), String> {
    let mut batches = history(dir)?;
    batches.pop();
    save_history(dir, &batches)
}

/// Writes the log, or removes it once it lists nothing. The text goes to a
/// temporary name first, so a reader never sees half a file.
fn save_history(dir: &Path, batches: &[Batch]) -> Result<(), String> {
    let path = dir.join(LOG);
    if batches.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(format!("{LOG}: {err}")),
            _ => Ok(()),
        };
    }
    let renamings: Vec<serde_json::Value> = batches
        .iter()
        .map(|batch| {
            let files: Vec<serde_json::Value> =
                batch.files.iter().map(|(from, to)| serde_json::json!({ "from": from, "to": to })).collect();
            serde_json::json!({ "at": batch.at, "files": files })
        })
        .collect();
    let doc = serde_json::json!({
        "about": "Files Picasso renamed in this folder, oldest first. Undo in Rename files puts back the newest.",
        "renamings": renamings,
    });
    let text = serde_json::to_string_pretty(&doc).map_err(|err| err.to_string())?;
    let temporary = dir.join(format!("{LOG}.{}.tmp", std::process::id()));
    std::fs::write(&temporary, text + "\n").map_err(|err| format!("{LOG}: {err}"))?;
    std::fs::rename(&temporary, &path).map_err(|err| {
        let _ = std::fs::remove_file(&temporary);
        format!("{LOG}: {err}")
    })
}

/// Renames a file, but never over another: `std::fs::rename` would replace it.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if to.symlink_metadata().is_ok() {
        return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, tr("The name is taken by another file")));
    }
    std::fs::rename(from, to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn file(id: i64, name: &str, taken: &str) -> File {
        let time = chrono::NaiveDateTime::parse_from_str(taken, "%Y-%m-%d %H:%M:%S").unwrap();
        let taken_at = chrono::Local.from_local_datetime(&time).earliest().unwrap().timestamp();
        File { id, name: name.into(), taken_at, modified_at: taken_at + 60, camera: "Sony ILCE-7M3".into() }
    }

    fn names(pattern: &str, files: &[File], others: &[&str]) -> Vec<Outcome> {
        let others: Vec<String> = others.iter().map(|name| name.to_string()).collect();
        plan(pattern, DATE_FORMAT, files, &others).unwrap()
    }

    fn rename(name: &str) -> Outcome {
        Outcome::Rename(name.into())
    }

    #[test]
    fn names_follow_the_exiftool_pattern_with_copy_numbers_for_the_same_second() {
        let files = [
            file(1, "DSC0001.JPG", "2023-05-17 14:30:05"),
            file(2, "DSC0002.JPG", "2023-05-17 14:30:05"),
            file(3, "DSC0003.JPG", "2023-05-17 14:30:05"),
            file(4, "clip.MOV", "2023-05-18 09:00:00"),
        ];
        assert_eq!(
            names(PATTERN, &files, &[]),
            [rename("2023.05.17-14h30m05s.jpg"), rename("2023.05.17-14h30m05s-001.jpg"), rename("2023.05.17-14h30m05s-002.jpg"), rename("2023.05.18-09h00m00s.mov")]
        );
        assert_eq!(names("%.c_%uf.%e", &files[..2], &[]), [rename("0_DSC0001.JPG"), rename("0_DSC0002.JPG")]);
        assert_eq!(names("x%.c.%le", &files[..2], &[]), [rename("x0.jpg"), rename("x1.jpg")]);
        assert_eq!(names("${Model}_%lf%+2c.%le", &files[..1], &[]), [rename("Sony ILCE-7M3_dsc0001.jpg")]);
        assert_eq!(names("$FileModifyDate.%le", &files[..1], &[]), [rename("2023.05.17-14h31m05s.jpg")]);
    }

    #[test]
    fn renaming_twice_changes_nothing_and_other_files_keep_their_names() {
        let files = [file(1, "2023.05.17-14h30m05s-001.jpg", "2023-05-17 14:30:05"), file(2, "IMG_9.jpg", "2023-05-17 14:30:05")];
        assert_eq!(names(PATTERN, &files, &[]), [Outcome::Same, rename("2023.05.17-14h30m05s.jpg")]);
        assert_eq!(names(PATTERN, &files, &["2023.05.17-14h30m05s.jpg"]), [Outcome::Same, rename("2023.05.17-14h30m05s-002.jpg")]);
        let upper = [file(1, "2023.05.17-14h30m05s.JPG", "2023-05-17 14:30:05")];
        assert_eq!(names(PATTERN, &upper, &[]), [rename("2023.05.17-14h30m05s.jpg")], "only the case of the extension changes");
        assert_eq!(
            names("${DateTimeOriginal}.%le", &files, &[]),
            [rename("2023.05.17-14h30m05s.jpg"), Outcome::Fails(tr("The name is taken by another file").into())],
            "without %c a second file can't have the name"
        );
    }

    #[test]
    fn bad_patterns_and_names_are_reported_before_anything_changes() {
        let files = [file(1, "a.jpg", "2023-05-17 14:30:05")];
        assert!(plan("${Lens}.%le", DATE_FORMAT, &files, &[]).unwrap_err().contains("${Lens}"));
        assert!(plan("${DateTimeOriginal.%le", DATE_FORMAT, &files, &[]).is_err());
        assert!(plan("%q.%le", DATE_FORMAT, &files, &[]).unwrap_err().contains("%q"));
        assert!(plan(PATTERN, "%Y-%Q", &files, &[]).is_err());
        assert!(plan("%f.%le", "%Y-%Q", &files, &[]).is_ok(), "the date format matters only with a date");
        let fails = |pattern: &str| matches!(names(pattern, &files, &[]).as_slice(), [Outcome::Fails(_)]);
        assert!(fails("${DateTimeOriginal}"), "the extension is gone");
        assert!(fails("%f.png"), "another extension");
        assert!(fails(".%le"));
        assert!(plan("%f%%.%le", DATE_FORMAT, &files, &[]).unwrap() == [rename("a%.jpg")]);
        let colon = plan("${DateTimeOriginal}.%le", "%H:%M", &files, &[]).unwrap();
        assert!(matches!(colon.as_slice(), [Outcome::Fails(_)]), "a colon can't be in a name");
        let unknown = File { camera: String::new(), ..file(2, "b.jpg", "2023-05-17 14:30:05") };
        assert!(matches!(names("${Model}.%le", &[unknown], &[]).as_slice(), [Outcome::Fails(_)]));
    }

    #[test]
    fn the_folder_lists_its_renamings_until_they_are_undone() {
        let dir = std::env::temp_dir().join(format!("picasso-rename-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pair = |from: &str, to: &str| (from.to_string(), to.to_string());
        assert_eq!(history(&dir).unwrap(), []);
        record(&dir, &[pair("a.jpg", "b.jpg")]).unwrap();
        record(&dir, &[pair("b.jpg", "c.jpg"), pair("x.jpg", "y.jpg")]).unwrap();
        let listed = history(&dir).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[1].files, [pair("b.jpg", "c.jpg"), pair("x.jpg", "y.jpg")]);
        forget_last(&dir).unwrap();
        assert_eq!(history(&dir).unwrap()[0].files, [pair("a.jpg", "b.jpg")]);
        forget_last(&dir).unwrap();
        assert!(!dir.join(LOG).exists(), "an empty list goes away");

        std::fs::write(dir.join(LOG), "not json").unwrap();
        assert!(record(&dir, &[pair("a.jpg", "b.jpg")]).is_err());
        assert_eq!(std::fs::read_to_string(dir.join(LOG)).unwrap(), "not json", "a list it can't read is kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_swap_names_and_nothing_is_renamed_over_another_file() {
        let dir = std::env::temp_dir().join(format!("picasso-rename-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.jpg"), "A").unwrap();
        std::fs::write(dir.join("b.jpg"), "B").unwrap();
        std::fs::write(dir.join("c.jpg"), "C").unwrap();
        let pair = |from: &str, to: &str| (from.to_string(), to.to_string());
        apply(&dir, &[pair("a.jpg", "b.jpg"), pair("b.jpg", "a.jpg")]).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a.jpg")).unwrap(), "B");
        assert_eq!(std::fs::read_to_string(dir.join("b.jpg")).unwrap(), "A");

        let err = apply(&dir, &[pair("a.jpg", "d.jpg"), pair("b.jpg", "c.jpg")]).unwrap_err();
        assert!(err.contains("c.jpg"));
        assert_eq!(std::fs::read_to_string(dir.join("a.jpg")).unwrap(), "B", "everything went back");
        assert_eq!(std::fs::read_to_string(dir.join("c.jpg")).unwrap(), "C");
        assert!(!dir.join("d.jpg").exists());
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|entry| entry.file_name()).collect();
        assert_eq!(left.len(), 3, "no temporary names are left: {left:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
