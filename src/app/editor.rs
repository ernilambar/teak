//! Editor commands the engine does not provide, plus the handler glue that
//! applies them to the focused editor.
//!
//! The line arithmetic lives in free functions over [`LineBuffer`] so it can be
//! unit tested without a window. Each command computes a single byte range to
//! replace and a caret offset; the handler then drives the engine through its
//! public API: select the range, `replace` it, move the caret.

use std::ops::Range;

use gpui_kit::base::input::{
    AddCursorAbove as EngineAddCursorAbove, AddCursorBelow as EngineAddCursorBelow,
};
use gpui_kit::component::{
    WindowExt as _,
    input::{EditorState, Input, InputState, Position, RopeExt as _},
    notification::Notification,
};
use gpui_kit::*;

use crate::{
    AddCursorAbove, AddCursorBelow, AddNextOccurrence, ConvertToCamelCase, ConvertToConstantCase,
    ConvertToKebabCase, ConvertToLowercase, ConvertToPascalCase, ConvertToSnakeCase,
    ConvertToTitleCase, ConvertToUppercase, CopyLineDown, CopyLineUp, DeleteLine,
    FindAndReplaceInEditor, FindInEditor, FindNextInEditor, FindPreviousInEditor, GoToLine,
    InsertLineAbove, InsertLineBelow, JoinLines, MoveLineDown, MoveLineUp, SelectLine,
    SplitSelectionIntoLines, ToggleLineComment, TrimTrailingWhitespace,
};

use super::TeakApp;

/// A single replacement to apply to the document.
struct Commit {
    /// Byte range in the current text to replace.
    range: Range<usize>,
    replacement: String,
    /// Caret byte offset in the text *after* the replacement is applied.
    cursor: usize,
}

/// The document viewed as lines. Line contents keep a trailing `\r` so CRLF
/// files round-trip, but drop the `\n`. `starts[i]` is the byte offset where
/// line `i` begins.
pub(super) struct LineBuffer<'a> {
    text: &'a str,
    starts: Vec<usize>,
    pub(super) lines: Vec<&'a str>,
    pub(super) trailing_newline: bool,
}

impl<'a> LineBuffer<'a> {
    pub(super) fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        for (i, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(i + 1);
            }
        }

        let trailing_newline = text.ends_with('\n');
        // A trailing `\n` does not start a new line; drop the empty line it
        // would otherwise create so the last content line stays the last line.
        if trailing_newline && starts.len() > 1 {
            starts.pop();
        }

        let lines = (0..starts.len())
            .map(|i| {
                let start = starts[i];
                let end = starts.get(i + 1).copied().unwrap_or(text.len());
                let end = if end > start && text.as_bytes()[end - 1] == b'\n' {
                    end - 1
                } else {
                    end
                };
                &text[start..end]
            })
            .collect();

        Self {
            text,
            starts,
            lines,
            trailing_newline,
        }
    }

    fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn is_last(&self, line: usize) -> bool {
        line + 1 == self.lines.len()
    }

    /// Whether line `line` is followed by a line break in the document.
    fn has_newline(&self, line: usize) -> bool {
        !self.is_last(line) || self.trailing_newline
    }

    fn start(&self, line: usize) -> usize {
        self.starts[line]
    }

    /// Byte offset just past line `line`, including its line break if present.
    fn end(&self, line: usize) -> usize {
        self.start(line) + self.lines[line].len() + usize::from(self.has_newline(line))
    }

    /// Index of the line containing byte offset `offset`.
    fn index_at(&self, offset: usize) -> usize {
        self.starts.partition_point(|&start| start <= offset) - 1
    }

    /// The line terminator the document uses, inferred from its first line
    /// break. Falls back to `\n` for documents with no break.
    fn line_ending(&self) -> &'static str {
        match self.text.find('\n') {
            Some(i) if i > 0 && self.text.as_bytes()[i - 1] == b'\r' => "\r\n",
            _ => "\n",
        }
    }

    /// The first and last lines touched by `selection`. An empty selection
    /// touches the line under the caret.
    fn touched(&self, selection: &Range<usize>) -> (usize, usize) {
        let first = self.index_at(selection.start);
        let last_offset = if selection.end > selection.start {
            selection.end - 1
        } else {
            selection.start
        };
        (first, self.index_at(last_offset))
    }
}

/// Joins line contents back into text, adding a final line break when asked.
pub(super) fn render(lines: &[&str], ends_with_newline: bool) -> String {
    let mut out = lines.join("\n");
    if ends_with_newline && !lines.is_empty() {
        out.push('\n');
    }
    out
}

/// Leading spaces and tabs of a line.
fn leading_indent(line: &str) -> &str {
    let rest = line.trim_start_matches([' ', '\t']);
    &line[..line.len() - rest.len()]
}

/// Deletes every line touched by the selection. When the last line is removed
/// and it had no trailing newline, the preceding newline goes with it so the
/// file does not keep a trailing blank line.
fn delete_lines(buf: &LineBuffer, selection: &Range<usize>) -> Commit {
    let (first, last) = buf.touched(selection);
    let mut range = buf.start(first)..buf.end(last);
    if range.end == buf.text.len() && !buf.has_newline(last) && range.start > 0 {
        // Drop the break that precedes the removed line so the file does not
        // keep a trailing blank line. The break is `\n` or `\r\n`.
        range.start -= 1;
        if range.start > 0 && buf.text.as_bytes()[range.start - 1] == b'\r' {
            range.start -= 1;
        }
    }
    Commit {
        cursor: range.start,
        range,
        replacement: String::new(),
    }
}

/// Joins the lines touched by the selection into one, or the caret's line with
/// the next when the selection covers a single line. Each break and the
/// whitespace around it become one space, or nothing next to an empty line.
/// The caret lands at the last join, before its space. Returns `None` on the last line.
fn join_lines(buf: &LineBuffer, selection: &Range<usize>) -> Option<Commit> {
    let (first, mut last) = buf.touched(selection);
    if first == last {
        if buf.is_last(first) {
            return None;
        }
        last += 1;
    }

    let mut joined = buf.lines[first]
        .trim_end_matches('\r')
        .trim_end_matches([' ', '\t'])
        .to_string();
    let mut cursor = joined.len();
    for (i, line) in buf.lines[first + 1..=last].iter().enumerate() {
        let line = line.trim_end_matches('\r');
        let line = line.trim_start_matches([' ', '\t']);
        // The last line keeps its trailing whitespace; it is not a join point.
        let line = if first + 1 + i == last {
            line
        } else {
            line.trim_end_matches([' ', '\t'])
        };
        cursor = joined.len();
        if !joined.is_empty() && !line.is_empty() {
            joined.push(' ');
        }
        joined.push_str(line);
    }

    // Stop before the last line's `\r` so a CRLF break stays intact.
    let start = buf.start(first);
    let end = buf.start(last) + buf.lines[last].trim_end_matches('\r').len();
    Some(Commit {
        range: start..end,
        replacement: joined,
        cursor: start + cursor,
    })
}

/// Moves the selected lines one line up or down. Returns `None` at the edge.
fn move_lines(buf: &LineBuffer, selection: &Range<usize>, up: bool) -> Option<Commit> {
    let (first, last) = buf.touched(selection);
    let moved = &buf.lines[first..=last];
    let caret_in_block = selection.start - buf.start(first);

    if up {
        if first == 0 {
            return None;
        }
        let mut lines: Vec<&str> = moved.to_vec();
        lines.push(buf.lines[first - 1]);
        let range = buf.start(first - 1)..buf.end(last);
        let replacement = render(&lines, buf.has_newline(last));
        Some(Commit {
            cursor: range.start + caret_in_block,
            range,
            replacement,
        })
    } else {
        if last + 1 >= buf.line_count() {
            return None;
        }
        let next = buf.lines[last + 1];
        let mut lines: Vec<&str> = Vec::with_capacity(moved.len() + 1);
        lines.push(next);
        lines.extend_from_slice(moved);
        let range = buf.start(first)..buf.end(last + 1);
        // `next` always gains a line break: the moved block follows it.
        let prefix = render(&[next], true).len();
        let replacement = render(&lines, buf.has_newline(last + 1));
        Some(Commit {
            cursor: range.start + prefix + caret_in_block,
            range,
            replacement,
        })
    }
}

/// Duplicates the selected lines. Copy up and copy down produce the same text
/// and both leave the caret on the lower of the two copies.
fn duplicate_lines(buf: &LineBuffer, selection: &Range<usize>) -> Commit {
    let (first, last) = buf.touched(selection);
    let moved = &buf.lines[first..=last];
    let caret_in_block = selection.start - buf.start(first);

    let mut lines: Vec<&str> = Vec::with_capacity(moved.len() * 2);
    lines.extend_from_slice(moved);
    lines.extend_from_slice(moved);

    let range = buf.start(first)..buf.end(last);
    let replacement = render(&lines, buf.has_newline(last));
    // The first copy is always followed by the second, so it gets a separator
    // even when the source line ended the file without one.
    let first_copy = render(moved, true).len();
    Commit {
        cursor: range.start + first_copy + caret_in_block,
        range,
        replacement,
    }
}

/// Inserts an empty line above or below the selection, keeping its indentation.
fn insert_line(buf: &LineBuffer, selection: &Range<usize>, above: bool) -> Commit {
    let (first, last) = buf.touched(selection);
    let nl = buf.line_ending();

    if above {
        let at = buf.start(first);
        let indent = leading_indent(buf.lines[first]);
        return Commit {
            range: at..at,
            replacement: format!("{indent}{nl}"),
            cursor: at + indent.len(),
        };
    }

    let at = buf.end(last);
    let indent = leading_indent(buf.lines[last]);
    if buf.has_newline(last) {
        // There is already a break after the line; open a new one before it.
        Commit {
            range: at..at,
            replacement: format!("{indent}{nl}"),
            cursor: at + indent.len(),
        }
    } else {
        // The line ends the file; start a fresh one after it.
        Commit {
            range: at..at,
            replacement: format!("{nl}{indent}"),
            cursor: at + nl.len() + indent.len(),
        }
    }
}

/// The full-line range the selection should expand to for "select line".
fn select_line_range(buf: &LineBuffer, selection: &Range<usize>) -> Range<usize> {
    let (first, last) = buf.touched(selection);
    buf.start(first)..buf.end(last)
}

/// Splits every non-empty selection into one selection per line it touches,
/// excluding line breaks. Empty selections stay as they are. Returns `None`
/// when no selection spans more than one line.
fn split_into_lines(buf: &LineBuffer, selections: &[Range<usize>]) -> Option<Vec<Range<usize>>> {
    let mut out = Vec::new();
    let mut split = false;
    for selection in selections {
        let (first, last) = buf.touched(selection);
        if selection.is_empty() || first == last {
            out.push(selection.clone());
            continue;
        }
        split = true;
        for line in first..=last {
            let start = buf.start(line);
            let end = start + buf.lines[line].trim_end_matches('\r').len();
            let sel_end = selection.end.min(end);
            out.push(selection.start.max(start).min(sel_end)..sel_end);
        }
    }
    if !split {
        return None;
    }
    out.sort_by_key(|range| range.start);
    Some(out)
}

/// Characters that make up a word for "add next occurrence". Non-ASCII
/// characters other than whitespace count, so scripts with combining marks
/// (Devanagari matras and virama) stay whole.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || (!c.is_ascii() && !c.is_whitespace())
}

/// The word touching byte offset `offset`, preferring the one after it.
fn word_at(text: &str, offset: usize) -> Option<Range<usize>> {
    let start = text[..offset]
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_word_char(c))
        .last()
        .map_or(offset, |(i, _)| i);
    let end = offset
        + text[offset..]
            .char_indices()
            .find(|&(_, c)| !is_word_char(c))
            .map_or(text.len() - offset, |(i, _)| i);
    (start < end).then_some(start..end)
}

/// Adds the next occurrence of the current selection (`⌘D`). `selections` lists
/// the active selection first and the result keeps that order. An empty active
/// selection expands to the word under it; otherwise the next match of the
/// active selection's text after the last selection in the document, wrapping
/// around and skipping matches already selected, is appended. Returns `None`
/// when there is nothing to add.
fn add_next_occurrence(text: &str, selections: &[Range<usize>]) -> Option<Vec<Range<usize>>> {
    let active = selections.first()?;
    if active.is_empty() {
        let word = word_at(text, active.start)?;
        let mut out = vec![word];
        out.extend_from_slice(&selections[1..]);
        return Some(out);
    }

    let needle = &text[active.clone()];
    let taken = |found: &Range<usize>| {
        selections
            .iter()
            .any(|s| found.start < s.end && s.start < found.end)
    };
    let search = |mut from: usize| {
        while let Some(i) = text[from..].find(needle) {
            let found = from + i..from + i + needle.len();
            if !taken(&found) {
                return Some(found);
            }
            from = found.end;
        }
        None
    };
    let last_end = selections.iter().map(|s| s.end).max()?;
    let found = search(last_end).or_else(|| search(0))?;

    let mut out = selections.to_vec();
    out.push(found);
    Some(out)
}

/// How a language comments out code.
#[derive(Clone, Copy)]
enum CommentStyle {
    Line(&'static str),
    Block {
        open: &'static str,
        close: &'static str,
    },
}

/// The comment syntax for an app language name, or `None` when the language has
/// no comments (plain text).
fn comment_style(language: &str) -> Option<CommentStyle> {
    match language {
        "Rust" | "JavaScript" | "TypeScript" | "TSX" | "PHP" | "JSON" => {
            Some(CommentStyle::Line("//"))
        }
        "Python" | "TOML" | "YAML" | "Shell" => Some(CommentStyle::Line("#")),
        "HTML" | "XML" | "Markdown" => Some(CommentStyle::Block {
            open: "<!--",
            close: "-->",
        }),
        "CSS" => Some(CommentStyle::Block {
            open: "/*",
            close: "*/",
        }),
        _ => None,
    }
}

fn toggle_comment(buf: &LineBuffer, selection: &Range<usize>, style: CommentStyle) -> Commit {
    let (first, last) = buf.touched(selection);
    let mut lines: Vec<String> = buf.lines[first..=last]
        .iter()
        .map(|line| (*line).to_string())
        .collect();

    match style {
        CommentStyle::Line(token) => {
            // Comment when any line still lacks the token; otherwise uncomment.
            let commented = buf.lines[first..=last]
                .iter()
                .all(|line| line.trim().is_empty() || line.trim_start().starts_with(token));

            for line in &mut lines {
                if line.trim().is_empty() {
                    continue;
                }
                let (indent, rest) = split_indent(line);
                if commented {
                    let rest = rest.strip_prefix(token).unwrap_or(rest);
                    let rest = rest.strip_prefix(' ').unwrap_or(rest);
                    *line = format!("{indent}{rest}");
                } else {
                    *line = format!("{indent}{token} {rest}");
                }
            }
        }
        CommentStyle::Block { open, close } => {
            let commented = buf.lines[first].trim_start().starts_with(open);
            if commented {
                let (indent, rest) = split_indent(&lines[0]);
                if let Some(rest) = rest.strip_prefix(open) {
                    let rest = rest.strip_prefix(' ').unwrap_or(rest);
                    lines[0] = format!("{indent}{rest}");
                }
                let end = lines.len() - 1;
                let suffixed = format!(" {close}");
                if let Some(rest) = lines[end].strip_suffix(&suffixed) {
                    lines[end] = rest.to_string();
                } else if let Some(rest) = lines[end].strip_suffix(close) {
                    lines[end] = rest.to_string();
                }
            } else {
                let (indent, rest) = split_indent(&lines[0]);
                lines[0] = format!("{indent}{open} {rest}");
                let end = lines.len() - 1;
                lines[end].push(' ');
                lines[end].push_str(close);
            }
        }
    }

    let range = buf.start(first)..buf.end(last);
    let locations: Vec<&str> = lines.iter().map(String::as_str).collect();
    let replacement = render(&locations, buf.has_newline(last));

    // The caret sits on the first line; shift it by that line's length change.
    let caret_in_line = selection.start - buf.start(first);
    let delta = lines[0].len() as isize - buf.lines[first].len() as isize;
    let caret = (caret_in_line as isize + delta).clamp(0, lines[0].len() as isize) as usize;

    Commit {
        cursor: range.start + caret,
        range,
        replacement,
    }
}

/// How a convert-case command rewrites its text.
#[derive(Clone, Copy)]
enum CaseKind {
    Upper,
    Lower,
    Title,
    Camel,
    Pascal,
    Snake,
    Kebab,
    Constant,
}

/// Whether `c` belongs to a word for case conversion. Non-ASCII characters
/// count unless they are whitespace, so scripts with combining marks (Devanagari
/// matras and virama) stay whole. Unlike [`is_word_char`], `_` separates words
/// here: it is the input and output of the snake and constant styles.
fn is_case_word_char(c: char) -> bool {
    c.is_alphanumeric() || (!c.is_ascii() && !c.is_whitespace())
}

/// Splits `text` into words for the identifier styles. Any other character
/// separates words, and so does a case change, except where it ends an acronym
/// (`HTTPServer` is `HTTP` + `Server`).
fn words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut previous: Option<char> = None;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if !is_case_word_char(c) {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            previous = None;
            continue;
        }
        let boundary = previous.is_some_and(|previous| {
            (previous.is_lowercase() && c.is_uppercase())
                || (previous.is_numeric() && c.is_uppercase())
                || (previous.is_uppercase()
                    && c.is_uppercase()
                    && chars.peek().is_some_and(|next| next.is_lowercase()))
        });
        if boundary {
            words.push(std::mem::take(&mut current));
        }
        current.push(c);
        previous = Some(c);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// `word` with its first character uppercased and the rest lowercased.
fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    first
        .to_uppercase()
        .chain(chars.as_str().to_lowercase().chars())
        .collect()
}

/// Rewrites `text` in `kind`. Uppercase and lowercase keep the separators the
/// user typed; every other style rebuilds the words the tokenizer finds.
fn convert(text: &str, kind: CaseKind) -> String {
    if matches!(kind, CaseKind::Upper) {
        return text.to_uppercase();
    }
    if matches!(kind, CaseKind::Lower) {
        return text.to_lowercase();
    }

    let words = words(text);
    let cased = |upper: bool| -> Vec<String> {
        words
            .iter()
            .map(|word| {
                if upper {
                    word.to_uppercase()
                } else {
                    word.to_lowercase()
                }
            })
            .collect()
    };

    match kind {
        CaseKind::Title => words
            .iter()
            .map(|word| capitalize(word))
            .collect::<Vec<_>>()
            .join(" "),
        CaseKind::Camel => words
            .iter()
            .enumerate()
            .map(|(ix, word)| {
                if ix == 0 {
                    word.to_lowercase()
                } else {
                    capitalize(word)
                }
            })
            .collect(),
        CaseKind::Pascal => words.iter().map(|word| capitalize(word)).collect(),
        CaseKind::Snake => cased(false).join("_"),
        CaseKind::Kebab => cased(false).join("-"),
        CaseKind::Constant => cased(true).join("_"),
        CaseKind::Upper | CaseKind::Lower => unreachable!("handled above"),
    }
}

/// Converts every target range as one edit: the span from the first range to
/// the last, its new text, and where each converted range lands afterwards.
/// Returns `None` when the text does not change.
fn convert_selections(
    text: &str,
    targets: &[Range<usize>],
    kind: CaseKind,
) -> Option<(Range<usize>, String, Vec<Range<usize>>)> {
    let mut targets = targets.to_vec();
    targets.sort_by_key(|range| range.start);
    // Converting overlapping ranges twice would count the same bytes twice.
    targets.dedup_by(|next, kept| next.start < kept.end);
    let first = targets.first()?;
    let last = targets.last()?;
    let span = first.start..last.end;

    let mut replacement = String::with_capacity(span.len());
    let mut restored = Vec::with_capacity(targets.len());
    let mut cursor = span.start;
    for target in &targets {
        replacement.push_str(&text[cursor..target.start]);
        let start = span.start + replacement.len();
        replacement.push_str(&convert(&text[target.clone()], kind));
        restored.push(start..span.start + replacement.len());
        cursor = target.end;
    }
    replacement.push_str(&text[cursor..span.end]);

    (replacement != text[span.clone()]).then_some((span, replacement, restored))
}

/// Applies [`tidy_edit`] to `editor` as one undoable edit, keeping the caret.
fn tidy(
    editor: &Entity<EditorState>,
    trim: bool,
    final_newline: bool,
    window: &mut Window,
    cx: &mut App,
) {
    editor.update(cx, |state, cx| {
        let text = state.text().to_string();
        let Some((range, replacement)) = tidy_edit(&text, trim, final_newline) else {
            return;
        };
        let caret = state.cursor_position();
        state.set_selected_range(range, cx);
        state.replace(replacement, window, cx);
        // Back to the caret's line; trimming may have shortened it. Setting
        // the range directly avoids focusing a background tab's editor.
        let offset = state.text().position_to_offset(&caret);
        state.set_selected_range(offset..offset, cx);
    });
}

/// The single edit that strips trailing spaces and tabs from every line
/// and/or ends the text with a newline, or `None` when nothing changes. The
/// range covers only the span between the first and last change, so undo and
/// scrolling stay local.
fn tidy_edit(text: &str, trim: bool, final_newline: bool) -> Option<(Range<usize>, String)> {
    let mut tidy = String::with_capacity(text.len() + 1);
    if trim {
        for line in text.split_inclusive('\n') {
            let (body, ending) = match line.strip_suffix("\r\n") {
                Some(body) => (body, "\r\n"),
                None => match line.strip_suffix('\n') {
                    Some(body) => (body, "\n"),
                    None => (line, ""),
                },
            };
            tidy.push_str(body.trim_end_matches([' ', '\t']));
            tidy.push_str(ending);
        }
    } else {
        tidy.push_str(text);
    }
    if final_newline && !tidy.is_empty() && !tidy.ends_with('\n') {
        tidy.push('\n');
    }
    if tidy == text {
        return None;
    }
    Some(changed_span(text, &tidy))
}

/// The single replacement that turns `old` into `new`: the byte range of `old`
/// between their common prefix and suffix, and what goes there.
pub(super) fn changed_span(old: &str, new: &str) -> (Range<usize>, String) {
    // Both strings share these bytes, and a UTF-8 lead byte fixes how many
    // continuation bytes follow, so a char boundary in one is one in the other.
    let mut prefix = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let mut suffix = old
        .bytes()
        .rev()
        .zip(new.bytes().rev())
        .take(old.len().min(new.len()) - prefix)
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(old.len() - suffix) {
        suffix -= 1;
    }
    (
        prefix..old.len() - suffix,
        new[prefix..new.len() - suffix].to_string(),
    )
}

fn split_indent(line: &str) -> (&str, &str) {
    let indent = leading_indent(line);
    (indent, &line[indent.len()..])
}

impl TeakApp {
    /// Runs `edit` against the focused editor's text and selection and applies
    /// the resulting replacement, if any.
    fn apply_to_editor<F>(&mut self, window: &mut Window, cx: &mut Context<Self>, edit: F)
    where
        F: FnOnce(&str, &Range<usize>) -> Option<Commit>,
    {
        let Some(tab) = self.active() else {
            return;
        };
        let editor = tab.document.editor.clone();

        editor.update(cx, |state, cx| {
            let text = state.text().to_string();
            let selection = state.selected_range();
            let Some(commit) = edit(&text, &selection) else {
                return;
            };

            state.set_selected_range(commit.range, cx);
            state.replace(commit.replacement, window, cx);

            let offset = commit.cursor.min(state.text().len());
            let position = state.text().offset_to_position(offset);
            state.set_cursor_position(position, window, cx);
        });
    }

    /// Applies `editor.trim_trailing_whitespace` and `editor.ensure_final_newline`
    /// to the buffer before a save, as one undoable edit.
    pub(super) fn tidy_for_save(
        &mut self,
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let trim = self.settings.values.editor.trim_trailing_whitespace();
        let final_newline = self.settings.values.editor.ensure_final_newline();
        if !trim && !final_newline {
            return;
        }
        tidy(editor, trim, final_newline, window, cx);
    }

    pub(super) fn on_trim_trailing_whitespace(
        &mut self,
        _: &TrimTrailingWhitespace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = self.active() {
            tidy(&tab.document.editor.clone(), true, false, window, cx);
        }
    }

    /// Converts each selection, or the word under each caret when nothing is
    /// selected, to `kind` as one undoable edit.
    fn convert_case(&mut self, kind: CaseKind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active() else {
            return;
        };
        let editor = tab.document.editor.clone();
        editor.update(cx, |state, cx| {
            let text = state.text().to_string();
            let targets: Vec<Range<usize>> = state
                .selected_ranges()
                .into_iter()
                .map(|selection| {
                    if selection.is_empty() {
                        word_at(&text, selection.start).unwrap_or(selection)
                    } else {
                        selection
                    }
                })
                .collect();
            let Some((span, replacement, restored)) = convert_selections(&text, &targets, kind)
            else {
                return;
            };
            state.set_selected_range(span, cx);
            state.replace(replacement, window, cx);
            // Leave what was converted selected, so converting again from the
            // palette applies to the same text.
            let mut restored = restored.into_iter();
            if let Some(first) = restored.next() {
                state.set_selected_range(first, cx);
            }
            for range in restored {
                state.add_selection(range, cx);
            }
        });
    }

    pub(super) fn on_convert_uppercase(
        &mut self,
        _: &ConvertToUppercase,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.convert_case(CaseKind::Upper, window, cx);
    }

    pub(super) fn on_convert_lowercase(
        &mut self,
        _: &ConvertToLowercase,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.convert_case(CaseKind::Lower, window, cx);
    }

    pub(super) fn on_convert_title_case(
        &mut self,
        _: &ConvertToTitleCase,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.convert_case(CaseKind::Title, window, cx);
    }

    pub(super) fn on_convert_camel_case(
        &mut self,
        _: &ConvertToCamelCase,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.convert_case(CaseKind::Camel, window, cx);
    }

    pub(super) fn on_convert_pascal_case(
        &mut self,
        _: &ConvertToPascalCase,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.convert_case(CaseKind::Pascal, window, cx);
    }

    pub(super) fn on_convert_snake_case(
        &mut self,
        _: &ConvertToSnakeCase,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.convert_case(CaseKind::Snake, window, cx);
    }

    pub(super) fn on_convert_kebab_case(
        &mut self,
        _: &ConvertToKebabCase,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.convert_case(CaseKind::Kebab, window, cx);
    }

    pub(super) fn on_convert_constant_case(
        &mut self,
        _: &ConvertToConstantCase,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.convert_case(CaseKind::Constant, window, cx);
    }

    pub(super) fn on_toggle_line_comment(
        &mut self,
        _: &ToggleLineComment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let language = self.active_language();
        let Some(style) = comment_style(&language) else {
            return;
        };
        self.apply_to_editor(window, cx, move |text, selection| {
            let buf = LineBuffer::new(text);
            Some(toggle_comment(&buf, selection, style))
        });
    }

    pub(super) fn on_delete_line(
        &mut self,
        _: &DeleteLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_to_editor(window, cx, |text, selection| {
            let buf = LineBuffer::new(text);
            Some(delete_lines(&buf, selection))
        });
    }

    pub(super) fn on_join_lines(
        &mut self,
        _: &JoinLines,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_to_editor(window, cx, |text, selection| {
            join_lines(&LineBuffer::new(text), selection)
        });
    }

    pub(super) fn on_move_line_up(
        &mut self,
        _: &MoveLineUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_to_editor(window, cx, |text, selection| {
            move_lines(&LineBuffer::new(text), selection, true)
        });
    }

    pub(super) fn on_move_line_down(
        &mut self,
        _: &MoveLineDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_to_editor(window, cx, |text, selection| {
            move_lines(&LineBuffer::new(text), selection, false)
        });
    }

    pub(super) fn on_copy_line_up(
        &mut self,
        _: &CopyLineUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_to_editor(window, cx, |text, selection| {
            Some(duplicate_lines(&LineBuffer::new(text), selection))
        });
    }

    pub(super) fn on_copy_line_down(
        &mut self,
        _: &CopyLineDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_to_editor(window, cx, |text, selection| {
            Some(duplicate_lines(&LineBuffer::new(text), selection))
        });
    }

    pub(super) fn on_insert_line_above(
        &mut self,
        _: &InsertLineAbove,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_to_editor(window, cx, |text, selection| {
            Some(insert_line(&LineBuffer::new(text), selection, true))
        });
    }

    pub(super) fn on_insert_line_below(
        &mut self,
        _: &InsertLineBelow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_to_editor(window, cx, |text, selection| {
            Some(insert_line(&LineBuffer::new(text), selection, false))
        });
    }

    pub(super) fn on_select_line(
        &mut self,
        _: &SelectLine,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active() else {
            return;
        };
        let editor = tab.document.editor.clone();
        editor.update(cx, |state, cx| {
            let text = state.text().to_string();
            let range = select_line_range(&LineBuffer::new(&text), &state.selected_range());
            state.set_selected_range(range, cx);
        });
    }

    /// Runs `select` against the focused editor's text and selections (active
    /// first) and applies the selections it returns, if any. The first becomes
    /// the active selection; the view scrolls to the last.
    fn select_in_editor<F>(&mut self, cx: &mut Context<Self>, select: F)
    where
        F: FnOnce(&str, &[Range<usize>]) -> Option<Vec<Range<usize>>>,
    {
        let Some(tab) = self.active() else {
            return;
        };
        let editor = tab.document.editor.clone();
        editor.update(cx, |state, cx| {
            let text = state.text().to_string();
            let Some(ranges) = select(&text, &state.selected_ranges()) else {
                return;
            };
            let mut ranges = ranges.into_iter();
            if let Some(first) = ranges.next() {
                state.set_selected_range(first, cx);
            }
            for range in ranges {
                state.add_selection(range, cx);
            }
        });
    }

    pub(super) fn on_split_selection_into_lines(
        &mut self,
        _: &SplitSelectionIntoLines,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_in_editor(cx, |text, selections| {
            split_into_lines(&LineBuffer::new(text), selections)
        });
    }

    pub(super) fn on_add_next_occurrence(
        &mut self,
        _: &AddNextOccurrence,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_in_editor(cx, add_next_occurrence);
    }

    /// Dispatches an engine action at the focused editor.
    ///
    /// The engine's add-cursor listeners live on the editor element, so the
    /// editor has to own focus before the action is sent. `Window::dispatch_action`
    /// reads the focused element at call time, and `Window::focus` sets it
    /// synchronously, so focusing first is enough.
    fn dispatch_to_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        action: Box<dyn Action>,
    ) {
        let Some(tab) = self.active() else {
            return;
        };
        // A previewing tab has no rendered editor to receive the action.
        if tab.previewing().is_some() {
            return;
        }
        let editor = tab.document.editor.clone();
        editor.update(cx, |state, cx| state.focus(window, cx));
        window.dispatch_action(action, cx);
    }

    pub(super) fn on_add_cursor_above(
        &mut self,
        _: &AddCursorAbove,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_to_editor(window, cx, Box::new(EngineAddCursorAbove));
    }

    pub(super) fn on_add_cursor_below(
        &mut self,
        _: &AddCursorBelow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_to_editor(window, cx, Box::new(EngineAddCursorBelow));
    }

    /// The editor of the active tab, ready for the in-editor search panel.
    ///
    /// A Markdown or SVG preview replaces the editor with a rendered view, so
    /// searching first returns the tab to its source. Raster tabs hold no text.
    fn editor_for_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<EditorState>> {
        let ix = self.tabs.active_index()?;
        let tab = self.tabs.get_mut(ix)?;
        if tab.raster {
            window.push_notification(Notification::info("This image has no text to search"), cx);
            return None;
        }
        if tab.previewing().is_some()
            && let Some(preview) = tab.preview.take()
        {
            preview.release(cx);
        }
        let editor = tab.document.editor.clone();
        self.focus_tab(ix, window, cx);
        cx.notify();
        Some(editor)
    }

    /// Opens the engine's search panel on the active editor, optionally in
    /// replace mode.
    fn open_editor_search(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor_for_search(window, cx) else {
            return;
        };
        editor.update(cx, |state, cx| state.open_search(replace, cx));
    }

    pub(super) fn on_find_in_editor(
        &mut self,
        _: &FindInEditor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_editor_search(false, window, cx);
    }

    pub(super) fn on_find_and_replace_in_editor(
        &mut self,
        _: &FindAndReplaceInEditor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_editor_search(true, window, cx);
    }

    pub(super) fn on_find_next_in_editor(
        &mut self,
        _: &FindNextInEditor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_editor_search(true, window, cx);
    }

    pub(super) fn on_find_previous_in_editor(
        &mut self,
        _: &FindPreviousInEditor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_editor_search(false, window, cx);
    }

    /// Moves to the next or previous match. With no search running, this opens
    /// the panel so the user can type a query.
    fn step_editor_search(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editor_for_search(window, cx) else {
            return;
        };
        editor.update(cx, |state, cx| {
            if !state.search_session().is_active() {
                state.open_search(false, cx);
                return;
            }
            let found = if forward {
                state.next_search_match(cx)
            } else {
                state.previous_search_match(cx)
            };
            if found.is_none() {
                state.open_search(false, cx);
            }
        });
    }

    pub(super) fn on_go_to_line(
        &mut self,
        _: &GoToLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active() else {
            return;
        };
        let editor = tab.document.editor.clone();

        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Line number"));
        let value_input = input.clone();
        let focus_input = input.clone();

        window.open_alert_dialog(cx, move |alert, _window, _cx| {
            let value_input = value_input.clone();
            let editor = editor.clone();
            alert
                .confirm()
                .title("Go to Line")
                .description("Enter a line number")
                .child(div().w_full().child(Input::new(&input).w_full()))
                .ok_text("Go")
                .cancel_text("Cancel")
                .on_ok(move |_, window, cx| {
                    let Ok(line) = value_input.read(cx).value().trim().parse::<usize>() else {
                        return false;
                    };
                    if line == 0 {
                        return false;
                    }
                    editor.update(cx, |state, cx| {
                        let last = state.text().lines_len().saturating_sub(1);
                        let line = (line - 1).min(last);
                        state.set_cursor_position(Position::new(line as u32, 0), window, cx);
                    });
                    true
                })
        });

        window.defer(cx, move |window, cx| {
            focus_input.update(cx, |state, cx| state.focus(window, cx));
        });
    }
}

#[cfg(test)]
// Selection lists of one range read naturally as `[a..b]` in the assertions.
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use std::ops::Range;

    // Explicit imports: `use super::*` would pull in gpui-kit's `test` attribute
    // macro and shadow the built-in one, making `#[test]` recurse.
    use super::{
        CaseKind, CommentStyle, Commit, LineBuffer, add_next_occurrence, comment_style, convert,
        convert_selections, delete_lines, duplicate_lines, insert_line, join_lines, move_lines,
        select_line_range, split_into_lines, tidy_edit, toggle_comment, words,
    };

    /// Applies a commit to `text` the way the handler does.
    fn apply(text: &str, commit: &Commit) -> String {
        let mut out = String::from(&text[..commit.range.start]);
        out.push_str(&commit.replacement);
        out.push_str(&text[commit.range.end..]);
        out
    }

    fn at(offset: usize) -> Range<usize> {
        offset..offset
    }

    #[test]
    fn buffer_splits_lines_and_offsets() {
        let buf = LineBuffer::new("a\nbb\nccc");
        assert_eq!(buf.lines, ["a", "bb", "ccc"]);
        assert_eq!(buf.starts, [0, 2, 5]);
        assert_eq!(buf.end(0), 2);
        assert_eq!(buf.end(2), 8);
        assert!(!buf.trailing_newline);

        let buf = LineBuffer::new("a\r\nb\n");
        assert_eq!(buf.lines, ["a\r", "b"]);
        assert!(buf.trailing_newline);
        assert_eq!(buf.lines.len(), 2);
    }

    #[test]
    fn empty_document_is_one_line() {
        let buf = LineBuffer::new("");
        assert_eq!(buf.lines, [""]);
        assert_eq!(buf.index_at(0), 0);
        assert_eq!(buf.end(0), 0);
    }

    #[test]
    fn index_at_maps_offsets_to_lines() {
        let text = "a\nbb\nccc";
        let buf = LineBuffer::new(text);
        assert_eq!(buf.index_at(0), 0);
        assert_eq!(buf.index_at(1), 0);
        assert_eq!(buf.index_at(2), 1);
        assert_eq!(buf.index_at(4), 1);
        assert_eq!(buf.index_at(5), 2);
        assert_eq!(buf.index_at(text.len()), 2);
    }

    #[test]
    fn trailing_newline_does_not_add_a_line() {
        let text = "a\nb\n";
        let buf = LineBuffer::new(text);
        assert_eq!(buf.lines, ["a", "b"]);
        assert!(buf.trailing_newline);
        assert_eq!(buf.index_at(text.len()), 1);
        assert!(move_lines(&buf, &at(2), false).is_none());
    }

    #[test]
    fn delete_middle_line() {
        let text = "a\nb\nc";
        let commit = delete_lines(&LineBuffer::new(text), &at(2));
        assert_eq!(apply(text, &commit), "a\nc");
        assert_eq!(commit.cursor, 2);
    }

    #[test]
    fn delete_last_line_without_trailing_newline_drops_the_break() {
        let text = "a\nb";
        let commit = delete_lines(&LineBuffer::new(text), &at(2));
        assert_eq!(apply(text, &commit), "a");
    }

    #[test]
    fn delete_last_line_with_trailing_newline_keeps_the_break() {
        let text = "a\nb\n";
        let commit = delete_lines(&LineBuffer::new(text), &at(2));
        assert_eq!(apply(text, &commit), "a\n");
    }

    #[test]
    fn delete_last_line_crlf_drops_the_whole_break() {
        let text = "a\r\nb";
        let commit = delete_lines(&LineBuffer::new(text), &at(3));
        assert_eq!(apply(text, &commit), "a");
    }

    #[test]
    fn join_caret_line_with_next() {
        let text = "a  \n    b\nc";
        let commit = join_lines(&LineBuffer::new(text), &at(0)).unwrap();
        assert_eq!(apply(text, &commit), "a b\nc");
        assert_eq!(commit.cursor, 1);
    }

    #[test]
    fn join_selected_lines() {
        let text = "a\nb\nc\nd";
        let commit = join_lines(&LineBuffer::new(text), &(0..5)).unwrap();
        assert_eq!(apply(text, &commit), "a b c\nd");
        assert_eq!(commit.cursor, 3);
    }

    #[test]
    fn join_skips_space_next_to_empty_line() {
        let text = "a\n\nb";
        let commit = join_lines(&LineBuffer::new(text), &at(0)).unwrap();
        assert_eq!(apply(text, &commit), "a\nb");
        let commit = join_lines(&LineBuffer::new(text), &(0..4)).unwrap();
        assert_eq!(apply(text, &commit), "a b");
    }

    #[test]
    fn join_keeps_crlf_break_of_last_line() {
        let text = "a\r\nb\r\nc";
        let commit = join_lines(&LineBuffer::new(text), &at(0)).unwrap();
        assert_eq!(apply(text, &commit), "a b\r\nc");
    }

    #[test]
    fn join_on_last_line_does_nothing() {
        assert!(join_lines(&LineBuffer::new("a\nb"), &at(2)).is_none());
    }

    #[test]
    fn delete_selected_lines() {
        let text = "a\nb\nc\nd";
        let commit = delete_lines(&LineBuffer::new(text), &(2..5));
        assert_eq!(apply(text, &commit), "a\nd");
    }

    #[test]
    fn move_line_up_swaps_with_previous() {
        let text = "a\nb\nc";
        let commit = move_lines(&LineBuffer::new(text), &at(4), true).unwrap();
        assert_eq!(apply(text, &commit), "a\nc\nb");
    }

    #[test]
    fn move_line_up_at_top_is_a_noop() {
        assert!(move_lines(&LineBuffer::new("a\nb"), &at(0), true).is_none());
    }

    #[test]
    fn move_line_down_swaps_with_next() {
        let text = "a\nb\nc";
        let commit = move_lines(&LineBuffer::new(text), &at(0), false).unwrap();
        assert_eq!(apply(text, &commit), "b\na\nc");
        assert_eq!(commit.cursor, 2);
    }

    #[test]
    fn move_line_down_at_bottom_is_a_noop() {
        assert!(move_lines(&LineBuffer::new("a\nb"), &at(2), false).is_none());
    }

    #[test]
    fn move_block_preserves_lines() {
        let text = "a\nb\nc\nd";
        let commit = move_lines(&LineBuffer::new(text), &(2..5), true).unwrap();
        assert_eq!(apply(text, &commit), "b\nc\na\nd");
    }

    #[test]
    fn duplicate_line_appends_a_copy() {
        let text = "a\nb";
        let commit = duplicate_lines(&LineBuffer::new(text), &at(0));
        assert_eq!(apply(text, &commit), "a\na\nb");
    }

    #[test]
    fn duplicate_last_line_without_newline() {
        let text = "a\nb";
        let commit = duplicate_lines(&LineBuffer::new(text), &at(2));
        assert_eq!(apply(text, &commit), "a\nb\nb");
        assert_eq!(commit.cursor, 4);
    }

    #[test]
    fn insert_line_below() {
        let text = "a\nb";
        let commit = insert_line(&LineBuffer::new(text), &at(0), false);
        assert_eq!(apply(text, &commit), "a\n\nb");
        assert_eq!(commit.cursor, 2);
    }

    #[test]
    fn insert_line_below_preserves_indent() {
        let text = "  a\nb";
        let commit = insert_line(&LineBuffer::new(text), &(2..3), false);
        assert_eq!(apply(text, &commit), "  a\n  \nb");
        assert_eq!(commit.cursor, 6);
    }

    #[test]
    fn insert_line_below_at_end_of_file() {
        let text = "a";
        let commit = insert_line(&LineBuffer::new(text), &at(0), false);
        assert_eq!(apply(text, &commit), "a\n");
        assert_eq!(commit.cursor, 2);
    }

    #[test]
    fn insert_line_above() {
        let text = "a\nb";
        let commit = insert_line(&LineBuffer::new(text), &at(2), true);
        assert_eq!(apply(text, &commit), "a\n\nb");
        assert_eq!(commit.cursor, 2);
    }

    #[test]
    fn insert_line_below_uses_crlf() {
        let text = "a\r\nb";
        let commit = insert_line(&LineBuffer::new(text), &at(0), false);
        assert_eq!(apply(text, &commit), "a\r\n\r\nb");
        assert_eq!(commit.cursor, 3);
    }

    #[test]
    fn insert_line_above_uses_crlf() {
        let text = "a\r\nb";
        let commit = insert_line(&LineBuffer::new(text), &at(3), true);
        assert_eq!(apply(text, &commit), "a\r\n\r\nb");
        assert_eq!(commit.cursor, 3);
    }

    #[test]
    fn insert_line_below_at_end_of_crlf_file() {
        let text = "a\r\nb";
        let commit = insert_line(&LineBuffer::new(text), &at(4), false);
        assert_eq!(apply(text, &commit), "a\r\nb\r\n");
        assert_eq!(commit.cursor, 6);
    }

    #[test]
    fn insert_line_below_preserves_indent_in_crlf_file() {
        let text = "  a\r\nb";
        let commit = insert_line(&LineBuffer::new(text), &at(2), false);
        assert_eq!(apply(text, &commit), "  a\r\n  \r\nb");
        assert_eq!(commit.cursor, 7);
    }

    #[test]
    fn select_line_expands_to_full_lines() {
        let text = "a\nbb\nc";
        let range = select_line_range(&LineBuffer::new(text), &at(3));
        assert_eq!(range, 2..5);
    }

    #[test]
    fn toggle_line_comment_comments_then_uncomments() {
        let text = "let x = 1;\n";
        let buf = LineBuffer::new(text);
        let commit = toggle_comment(&buf, &at(0), CommentStyle::Line("//"));
        assert_eq!(apply(text, &commit), "// let x = 1;\n");

        let text = "// let x = 1;\n";
        let buf = LineBuffer::new(text);
        let commit = toggle_comment(&buf, &at(0), CommentStyle::Line("//"));
        assert_eq!(apply(text, &commit), "let x = 1;\n");
    }

    #[test]
    fn toggle_line_comment_keeps_indentation() {
        let text = "    let x = 1;\n";
        let buf = LineBuffer::new(text);
        let commit = toggle_comment(&buf, &at(4), CommentStyle::Line("//"));
        assert_eq!(apply(text, &commit), "    // let x = 1;\n");
    }

    #[test]
    fn toggle_line_comment_skips_blank_lines() {
        let text = "a\n\nb\n";
        let buf = LineBuffer::new(text);
        let commit = toggle_comment(&buf, &(0..4), CommentStyle::Line("//"));
        assert_eq!(apply(text, &commit), "// a\n\n// b\n");
    }

    #[test]
    fn toggle_line_comment_comments_mixed_selection() {
        let text = "// a\nb\n";
        let buf = LineBuffer::new(text);
        let commit = toggle_comment(&buf, &(0..6), CommentStyle::Line("//"));
        assert_eq!(apply(text, &commit), "// // a\n// b\n");
    }

    #[test]
    fn toggle_block_comment_wraps_single_line() {
        let text = "<div>\n";
        let buf = LineBuffer::new(text);
        let style = CommentStyle::Block {
            open: "<!--",
            close: "-->",
        };
        let commit = toggle_comment(&buf, &at(0), style);
        assert_eq!(apply(text, &commit), "<!-- <div> -->\n");

        let text = "<!-- <div> -->\n";
        let buf = LineBuffer::new(text);
        let commit = toggle_comment(&buf, &at(0), style);
        assert_eq!(apply(text, &commit), "<div>\n");
    }

    #[test]
    fn comment_style_covers_supported_languages() {
        assert!(matches!(
            comment_style("Rust"),
            Some(CommentStyle::Line("//"))
        ));
        assert!(matches!(
            comment_style("Python"),
            Some(CommentStyle::Line("#"))
        ));
        assert!(matches!(
            comment_style("HTML"),
            Some(CommentStyle::Block { .. })
        ));
        assert!(matches!(
            comment_style("YAML"),
            Some(CommentStyle::Line("#"))
        ));
        assert!(matches!(
            comment_style("Shell"),
            Some(CommentStyle::Line("#"))
        ));
        assert!(matches!(
            comment_style("TSX"),
            Some(CommentStyle::Line("//"))
        ));
        assert!(matches!(
            comment_style("CSS"),
            Some(CommentStyle::Block { .. })
        ));
        assert!(comment_style("Plain Text").is_none());
    }

    #[test]
    fn split_selection_into_lines_excludes_breaks() {
        let text = "ab\ncd\r\nef";
        let buf = LineBuffer::new(text);
        assert_eq!(split_into_lines(&buf, &[1..8]).unwrap(), [1..2, 3..5, 7..8]);
    }

    #[test]
    fn split_selection_keeps_empty_lines_and_carets() {
        let text = "a\n\nb";
        let buf = LineBuffer::new(text);
        assert_eq!(
            split_into_lines(&buf, &[at(0), 0..4]).unwrap(),
            [at(0), 0..1, 2..2, 3..4]
        );
    }

    #[test]
    fn split_selection_ending_at_line_start_skips_that_line() {
        let text = "a\nb\nc";
        let buf = LineBuffer::new(text);
        assert_eq!(split_into_lines(&buf, &[0..4]).unwrap(), [0..1, 2..3]);
    }

    #[test]
    fn split_single_line_selection_is_a_no_op() {
        let buf = LineBuffer::new("abc\ndef");
        assert!(split_into_lines(&buf, &[0..2, at(5)]).is_none());
    }

    #[test]
    fn next_occurrence_expands_caret_to_word() {
        assert_eq!(add_next_occurrence("foo bar", &[at(5)]).unwrap(), [4..7]);
        assert_eq!(add_next_occurrence("foo bar", &[at(3)]).unwrap(), [0..3]);
        assert!(add_next_occurrence("a  b", &[at(2)]).is_none());
    }

    #[test]
    fn next_occurrence_keeps_devanagari_words_whole() {
        let text = "नमस्ते संसार";
        let word = add_next_occurrence(text, &[at(3)]).unwrap();
        assert_eq!(&text[word[0].clone()], "नमस्ते");
    }

    #[test]
    fn next_occurrence_appends_following_match() {
        let text = "ab ab ab";
        assert_eq!(add_next_occurrence(text, &[0..2]).unwrap(), [0..2, 3..5]);
        assert_eq!(
            add_next_occurrence(text, &[0..2, 3..5]).unwrap(),
            [0..2, 3..5, 6..8]
        );
    }

    #[test]
    fn next_occurrence_searches_after_the_last_selection() {
        let text = "ab ab ab ab";
        assert_eq!(
            add_next_occurrence(text, &[3..5, 6..8]).unwrap(),
            [3..5, 6..8, 9..11]
        );
    }

    #[test]
    fn next_occurrence_wraps_and_skips_selected() {
        let text = "ab ab ab";
        assert_eq!(
            add_next_occurrence(text, &[3..5, 6..8]).unwrap(),
            [3..5, 6..8, 0..2]
        );
        assert!(add_next_occurrence(text, &[0..2, 3..5, 6..8]).is_none());
    }

    fn tidied(text: &str, trim: bool, final_newline: bool) -> String {
        match tidy_edit(text, trim, final_newline) {
            Some((range, replacement)) => {
                let mut out = String::from(&text[..range.start]);
                out.push_str(&replacement);
                out.push_str(&text[range.end..]);
                out
            }
            None => text.to_string(),
        }
    }

    #[test]
    fn tidy_trims_line_ends_and_keeps_crlf() {
        assert_eq!(
            tidied("a  \r\n\tb\t\n   \nc ", true, false),
            "a\r\n\tb\n\nc"
        );
        assert_eq!(tidied("é  \nü", true, false), "é\nü");
    }

    #[test]
    fn tidy_adds_one_final_newline() {
        assert_eq!(tidied("a", false, true), "a\n");
        assert_eq!(tidied("a  ", true, true), "a\n");
        assert!(
            tidy_edit("", false, true).is_none(),
            "an empty file stays empty"
        );
        assert!(tidy_edit("a\n", true, true).is_none());
    }

    #[test]
    fn tidy_replaces_only_the_changed_span() {
        let (range, replacement) = tidy_edit("keep\nx  \nkeep\n", true, false).unwrap();
        assert_eq!(range, 6..8);
        assert_eq!(replacement, "");
    }

    #[test]
    fn words_split_on_separators_and_case_changes() {
        assert_eq!(words("helloWorld"), ["hello", "World"]);
        assert_eq!(
            words("hello_world-two three"),
            ["hello", "world", "two", "three"]
        );
        assert_eq!(words("HTTPServer"), ["HTTP", "Server"]);
        assert_eq!(words("XMLHttpRequest"), ["XML", "Http", "Request"]);
        assert_eq!(words("version2Beta"), ["version2", "Beta"]);
        assert_eq!(words(""), Vec::<String>::new());
    }

    #[test]
    fn words_keep_non_cased_scripts_whole() {
        assert_eq!(words("नमस्ते संसार"), ["नमस्ते", "संसार"]);
    }

    #[test]
    fn convert_case_styles() {
        assert_eq!(convert("helloWorld", CaseKind::Upper), "HELLOWORLD");
        assert_eq!(convert("helloWorld", CaseKind::Lower), "helloworld");
        assert_eq!(convert("helloWorld", CaseKind::Title), "Hello World");
        assert_eq!(convert("helloWorld", CaseKind::Camel), "helloWorld");
        assert_eq!(convert("helloWorld", CaseKind::Pascal), "HelloWorld");
        assert_eq!(convert("helloWorld", CaseKind::Snake), "hello_world");
        assert_eq!(convert("helloWorld", CaseKind::Kebab), "hello-world");
        assert_eq!(convert("helloWorld", CaseKind::Constant), "HELLO_WORLD");
    }

    #[test]
    fn convert_case_round_trips_an_acronym() {
        let text = "XMLHttpRequest";
        assert_eq!(convert(text, CaseKind::Camel), "xmlHttpRequest");
        assert_eq!(convert(text, CaseKind::Pascal), "XmlHttpRequest");
        assert_eq!(convert(text, CaseKind::Snake), "xml_http_request");
        assert_eq!(convert(text, CaseKind::Kebab), "xml-http-request");
        assert_eq!(convert(text, CaseKind::Constant), "XML_HTTP_REQUEST");
    }

    #[test]
    fn upper_and_lower_keep_the_separators() {
        assert_eq!(
            convert("hello_world-two", CaseKind::Upper),
            "HELLO_WORLD-TWO"
        );
        assert_eq!(convert("Hello World", CaseKind::Lower), "hello world");
    }

    #[test]
    fn convert_selections_edits_one_span() {
        let text = "one two three";
        let (span, replacement, restored) =
            convert_selections(text, &[0..3, 8..13], CaseKind::Upper).unwrap();
        assert_eq!(span, 0..13);
        assert_eq!(replacement, "ONE two THREE");
        assert_eq!(restored, [0..3, 8..13]);
    }

    #[test]
    fn convert_selections_skips_overlaps_and_no_ops() {
        let text = "one two";
        let (span, replacement, restored) =
            convert_selections(text, &[0..3, 2..5], CaseKind::Upper).unwrap();
        assert_eq!(span, 0..3);
        assert_eq!(replacement, "ONE");
        assert_eq!(restored, [0..3]);
        assert!(convert_selections("ABC", &[0..3], CaseKind::Upper).is_none());
    }

    /// `secondary-f` opens the engine's search panel on the active editor, and
    /// `secondary-alt-f` opens it in replace mode.
    #[gpui_kit::test]
    async fn find_opens_the_in_editor_search_panel(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        use gpui_kit::test::TestWindowExt as _;

        use crate::app::test_support::open_app;

        let (handle, app) = open_app(cx);
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| {
                this.add_tab(None, "one two one".into(), window, cx);
            });
            window.render_frame(cx);
            window.press("secondary-f", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(handle, |_, window, cx| {
            let editor = app.read(cx).tabs.active().unwrap().document.editor.clone();
            let session = editor.read(cx).search_session().clone();
            assert!(session.open, "Find opens the panel");
            assert!(!session.replace_mode, "Find is not replace mode");

            window.press("secondary-alt-f", cx);
            window.render_frame(cx);
            assert!(
                editor.read(cx).search_session().replace_mode,
                "Find and Replace opens replace mode"
            );
        })
        .unwrap();
    }

    /// F3 and Shift+F3 step through the current session's matches.
    #[gpui_kit::test]
    async fn find_next_and_previous_step_through_matches(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        use gpui_kit::test::TestWindowExt as _;

        use crate::app::test_support::open_app;

        let (handle, app) = open_app(cx);
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| {
                this.add_tab(None, "one two one".into(), window, cx);
                let editor = this.tabs.active().unwrap().document.editor.clone();
                editor.update(cx, |state, cx| state.set_search_query("one", true, cx));
            });
            window.render_frame(cx);
            window.press("f3", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(handle, |_, window, cx| {
            let editor = app.read(cx).tabs.active().unwrap().document.editor.clone();
            assert_eq!(editor.read(cx).search_session().matcher.label(), "2/2");

            window.press("shift-f3", cx);
            window.render_frame(cx);
            assert_eq!(editor.read(cx).search_session().matcher.label(), "1/2");
        })
        .unwrap();
    }

    /// Find on a Markdown preview returns the tab to its source before
    /// searching.
    #[gpui_kit::test]
    async fn find_on_a_preview_returns_to_the_source(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        use gpui_kit::test::TestWindowExt as _;

        use crate::app::test_support::open_app;

        let (handle, app) = open_app(cx);
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| {
                this.add_document_tab("Doc", "Markdown", "# Title".into(), window, cx);
                let ix = this.tabs.active_index().unwrap();
                let preview =
                    crate::app::tabs::Preview::for_text("Markdown", "# Title", cx).unwrap();
                this.tabs.get_mut(ix).unwrap().preview = Some(preview);
                assert!(this.tabs.get(ix).unwrap().previewing().is_some());
            });
            let focus = app.read(cx).focus_handle.clone();
            window.focus(&focus, cx);
            window.render_frame(cx);
            window.press("secondary-f", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(handle, |_, _, cx| {
            let this = app.read(cx);
            let tab = this.tabs.active().unwrap();
            assert!(tab.previewing().is_none(), "preview is turned off");
            assert!(
                tab.document.editor.read(cx).search_session().open,
                "search opens on the source"
            );
        })
        .unwrap();
    }
}
