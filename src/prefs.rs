//! Editor preferences, persisted as a GKeyFile under the user config dir.

use gtk::prelude::*;
use gtk::glib;

#[derive(Clone, Copy, PartialEq)]
pub struct Prefs {
    pub line_numbers: bool,
    pub highlight_current_line: bool,
    pub auto_indent: bool,
    pub insert_spaces: bool,
    pub word_wrap: bool,
    pub dark_mode: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            line_numbers: true,
            highlight_current_line: true,
            auto_indent: true,
            insert_spaces: true,
            word_wrap: false,
            dark_mode: false,
        }
    }
}

const GROUP: &str = "editor";

impl Prefs {
    fn config_file() -> std::path::PathBuf {
        glib::user_config_dir().join("gedit-rs.ini")
    }

    pub fn load() -> Self {
        let mut p = Self::default();
        let kf = glib::KeyFile::new();
        if kf
            .load_from_file(Self::config_file(), glib::KeyFileFlags::NONE)
            .is_ok()
        {
            if let Ok(v) = kf.boolean(GROUP, "line-numbers") {
                p.line_numbers = v;
            }
            if let Ok(v) = kf.boolean(GROUP, "highlight-current-line") {
                p.highlight_current_line = v;
            }
            if let Ok(v) = kf.boolean(GROUP, "auto-indent") {
                p.auto_indent = v;
            }
            if let Ok(v) = kf.boolean(GROUP, "insert-spaces") {
                p.insert_spaces = v;
            }
            if let Ok(v) = kf.boolean(GROUP, "word-wrap") {
                p.word_wrap = v;
            }
            if let Ok(v) = kf.boolean(GROUP, "dark-mode") {
                p.dark_mode = v;
            }
        }
        p
    }

    pub fn save(&self) {
        let kf = glib::KeyFile::new();
        let _ = kf.set_boolean(GROUP, "line-numbers", self.line_numbers);
        let _ = kf.set_boolean(GROUP, "highlight-current-line", self.highlight_current_line);
        let _ = kf.set_boolean(GROUP, "auto-indent", self.auto_indent);
        let _ = kf.set_boolean(GROUP, "insert-spaces", self.insert_spaces);
        let _ = kf.set_boolean(GROUP, "word-wrap", self.word_wrap);
        let _ = kf.set_boolean(GROUP, "dark-mode", self.dark_mode);
        let _ = kf.save_to_file(Self::config_file());
    }

    /// Apply the color scheme globally (GTK setting, not per-view).
    /// `gtk::StyleManager` is not exposed by gtk4-rs 0.11, so set the
    /// GtkSettings `gtk-interface-color-scheme` enum property directly
    /// (GTK >= 4.20), falling back to the legacy boolean on older GTK.
    pub fn apply_color_scheme(&self) {
        let Some(settings) = gtk::Settings::default() else { return };
        let scheme = if self.dark_mode { "dark" } else { "default" };
        if let Some(pspec) = settings.find_property("gtk-interface-color-scheme") {
            let Some(class) = glib::EnumClass::with_type(pspec.value_type()) else { return };
            let Some(value) = class.value_by_nick(scheme) else { return };
            settings.set_property_from_value("gtk-interface-color-scheme", &value.to_value(&class));
        } else {
            settings.set_property("gtk-application-prefer-dark-theme", self.dark_mode);
        }
    }
}
