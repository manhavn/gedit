//! One open text file: a `GtkSourceView` editor bound to a `GtkSourceBuffer`,
//! plus its backing path (if any).

use gtk::prelude::*;
use sourceview::prelude::*;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::prefs::Prefs;

/// Style scheme for the editor under `dark`. GtkSourceView ships themed
/// pairs (`Yaru`/`Yaru-dark` on Ubuntu); fall back to any installed id that
/// does/doesn't end in `-dark`, and to `None` (buffer default) if nothing
/// matches.
fn style_scheme(dark: bool) -> Option<sourceview::StyleScheme> {
    let manager = sourceview::StyleSchemeManager::default();
    let preferred: &[&str] = if dark {
        &["Yaru-dark", "oblivion"]
    } else {
        &["Yaru", "classic"]
    };
    preferred
        .iter()
        .find_map(|id| manager.scheme(id))
        .or_else(|| {
            manager
                .scheme_ids()
                .into_iter()
                .find(|id| id.ends_with("-dark") == dark)
                .and_then(|id| manager.scheme(&id))
        })
}

/// Guess and apply the syntax language for `path` on `buffer`.
fn guess_language(buffer: &sourceview::Buffer, path: &Path) {
    let manager = sourceview::LanguageManager::default();
    let lang = manager.guess_language(Some(path), None);
    buffer.set_language(lang.as_ref());
}

pub struct Document {
    pub view: sourceview::View,
    pub buffer: sourceview::Buffer,
    pub file: Option<PathBuf>,
    /// Name shown while there is no file yet, e.g. "Untitled Document 1".
    /// Assigned by the caller so numbered tabs stay distinguishable.
    untitled_name: String,
}

impl Document {
    pub fn new(prefs: &Prefs, untitled_name: &str) -> Rc<RefCell<Self>> {
        let buffer = sourceview::Buffer::new(None::<&gtk::TextTagTable>);
        let view = sourceview::View::with_buffer(&buffer);
        view.set_monospace(true);
        view.set_show_line_numbers(prefs.line_numbers);
        view.set_highlight_current_line(prefs.highlight_current_line);
        view.set_auto_indent(prefs.auto_indent);
        view.set_insert_spaces_instead_of_tabs(prefs.insert_spaces);
        view.set_tab_width(4);
        view.set_wrap_mode(if prefs.word_wrap {
            gtk::WrapMode::Word
        } else {
            gtk::WrapMode::None
        });
        buffer.set_style_scheme(style_scheme(prefs.dark_mode).as_ref());
        Rc::new(RefCell::new(Self {
            view,
            buffer,
            file: None,
            untitled_name: untitled_name.to_string(),
        }))
    }

    /// Basename of the file, or the untitled name ("Untitled Document N").
    /// Read only after `load_file`, once `file` is set.
    pub fn display_name(&self) -> String {
        match &self.file {
            Some(p) => p
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.to_string_lossy().into_owned()),
            None => self.untitled_name.clone(),
        }
    }

    /// Absolute path, or the untitled name when the document was never saved.
    pub fn full_name(&self) -> String {
        match &self.file {
            Some(p) => p.to_string_lossy().into_owned(),
            None => self.untitled_name.clone(),
        }
    }

    /// Read `path` into the buffer, guess the syntax language, clear modified.
    pub fn load_file(&mut self, path: &Path) -> Result<(), String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {}", path.display(), e))?;
        let text =
            String::from_utf8(bytes).map_err(|_| format!("{} is not valid UTF-8", path.display()))?;
        self.buffer.set_text(&text);
        self.file = Some(path.to_path_buf());
        self.buffer.set_modified(false);
        guess_language(&self.buffer, path);
        Ok(())
    }

    /// Rebuild the document from the session cache. `text` is the cached
    /// buffer contents — the on-disk copy may be stale (unsaved edits) or
    /// gone (file deleted while the cache existed).
    pub fn restore(&mut self, path: Option<&Path>, text: &str, modified: bool) {
        self.buffer.set_text(text);
        self.file = path.map(Path::to_path_buf);
        if let Some(p) = path {
            guess_language(&self.buffer, p);
        }
        self.buffer.set_modified(modified);
    }

    /// Buffer contents as plain text.
    pub fn text(&self) -> String {
        let (start, end) = self.buffer.bounds();
        self.buffer.text(&start, &end, true).to_string()
    }

    /// Bind the document to `path` and clear the modified flag. Call after
    /// the buffer contents have been written to `path` — writing is the
    /// caller's job because it may need elevated privileges
    /// (`Win::save_doc_to`).
    pub fn mark_saved(&mut self, path: &Path) {
        self.file = Some(path.to_path_buf());
        self.buffer.set_modified(false);
    }

    pub fn apply_prefs(&self, prefs: &Prefs) {
        self.view.set_show_line_numbers(prefs.line_numbers);
        self.view
            .set_highlight_current_line(prefs.highlight_current_line);
        self.view.set_auto_indent(prefs.auto_indent);
        self.view.set_insert_spaces_instead_of_tabs(prefs.insert_spaces);
        self.view.set_wrap_mode(if prefs.word_wrap {
            gtk::WrapMode::Word
        } else {
            gtk::WrapMode::None
        });
        self.buffer
            .set_style_scheme(style_scheme(prefs.dark_mode).as_ref());
    }
}
