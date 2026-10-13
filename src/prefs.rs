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

    /// Apply the dark/light preference to the GTK chrome.
    ///
    /// `gtk-interface-color-scheme` (GTK >= 4.20) only mirrors the *system*
    /// preference for CSS media queries — setting it rethemes nothing
    /// (verified: pixel-identical snapshots with it forced to light; Yaru
    /// ships no media queries either). What works:
    ///
    /// - `gtk-application-prefer-dark-theme` (deprecated in 4.20, still
    ///   live) selects the theme's `-dark` variant when on;
    /// - switching `gtk-theme-name` between `<base>` and `<base>-dark` —
    ///   a system-wide `-dark` name (gnome-settings-daemon pushes
    ///   `gtk-theme=Yaru-dark` on a dark desktop) must be stripped for
    ///   light, and an explicit `-dark` name is what dark needs at runtime:
    ///   flipping only prefer-dark live reloads the base theme's
    ///   `gtk-dark.css` variant, which leaves the header bar background
    ///   unpainted (verified via snapshot: transparent to the backdrop),
    ///   while the `<base>-dark` theme directory loads fully opaque.
    pub fn apply_color_scheme(&self) {
        let Some(settings) = gtk::Settings::default() else { return };
        let theme: String = settings.property("gtk-theme-name");
        let base = theme.strip_suffix("-dark").unwrap_or(&theme).to_string();
        settings.set_property("gtk-application-prefer-dark-theme", self.dark_mode);
        if self.dark_mode {
            // Only switch to a `-dark` name that really exists; otherwise
            // keep the base and let prefer-dark fall back gracefully.
            let variant = format!("{base}-dark");
            if variant != theme && theme_exists(&variant) {
                settings.set_property("gtk-theme-name", &variant);
            }
        } else if base != theme {
            settings.set_property("gtk-theme-name", &base);
        }
    }
}

/// Whether `name` ships gtk-4.0 assets under the standard theme dirs.
fn theme_exists(name: &str) -> bool {
    std::iter::once(glib::user_data_dir())
        .chain(glib::system_data_dirs())
        .any(|dir| dir.join("themes").join(name).join("gtk-4.0").exists())
}
