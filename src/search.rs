//! Find & replace bar (GtkSearchBar + GtkSourceSearchContext), gedit-style:
//! Ctrl+F opens search, Ctrl+H adds the replace row, Enter / Shift+Enter
//! jump next / previous match with live highlighting.

use gtk::prelude::*;
use gtk::glib;
use sourceview::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

pub struct SearchUi {
    pub bar: gtk::SearchBar,
    entry: gtk::SearchEntry,
    replace_row: gtk::Box,
    replace_entry: gtk::SearchEntry,
    case_toggle: gtk::ToggleButton,
    regex_toggle: gtk::ToggleButton,
    settings: Rc<sourceview::SearchSettings>,
    context: RefCell<Option<sourceview::SearchContext>>,
    view: RefCell<Option<sourceview::View>>,
}

impl SearchUi {
    pub fn new() -> Rc<Self> {
        let settings = Rc::new(sourceview::SearchSettings::new());
        settings.set_wrap_around(true);

        let entry = gtk::SearchEntry::new();
        entry.set_placeholder_text(Some("Search"));
        entry.set_hexpand(true);

        let replace_entry = gtk::SearchEntry::new();
        replace_entry.set_placeholder_text(Some("Replace with"));
        replace_entry.set_hexpand(true);

        let case_toggle = gtk::ToggleButton::new();
        case_toggle.set_label("Aa");
        case_toggle.set_tooltip_text(Some("Match case"));
        let regex_toggle = gtk::ToggleButton::new();
        regex_toggle.set_label(".*");
        regex_toggle.set_tooltip_text(Some("Regular expression"));

        let replace_button = gtk::Button::with_label("Replace");
        let replace_all_button = gtk::Button::with_label("Replace All");

        let find_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        find_row.append(&entry);
        find_row.append(&case_toggle);
        find_row.append(&regex_toggle);

        let replace_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        replace_row.append(&replace_entry);
        replace_row.append(&replace_button);
        replace_row.append(&replace_all_button);
        replace_row.set_visible(false);

        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 6);
        vbox.append(&find_row);
        vbox.append(&replace_row);
        vbox.set_margin_top(6);
        vbox.set_margin_bottom(6);
        vbox.set_margin_start(12);
        vbox.set_margin_end(12);

        let bar = gtk::SearchBar::new();
        bar.set_child(Some(&vbox));

        let ui = Rc::new(Self {
            bar,
            entry: entry.clone(),
            replace_row: replace_row.clone(),
            replace_entry: replace_entry.clone(),
            case_toggle: case_toggle.clone(),
            regex_toggle: regex_toggle.clone(),
            settings,
            context: RefCell::new(None),
            view: RefCell::new(None),
        });

        // Settings toggles drive search-context behavior and re-run the search.
        let ui2 = ui.clone();
        case_toggle.connect_toggled(move |_| {
            ui2.settings.set_case_sensitive(ui2.case_toggle.is_active());
            ui2.step(false);
        });
        let ui2 = ui.clone();
        regex_toggle.connect_toggled(move |_| {
            ui2.settings.set_regex_enabled(ui2.regex_toggle.is_active());
            ui2.step(false);
        });

        let ui2 = ui.clone();
        entry.connect_search_changed(move |e| {
            ui2.settings.set_search_text(Some(e.text().as_str()));
            ui2.step(false);
        });
        let ui2 = ui.clone();
        entry.connect_activate(move |_| ui2.step(false));

        // Shift+Enter searches backwards.
        let ui2 = ui.clone();
        let shift_controller = gtk::EventControllerKey::new();
        shift_controller.connect_key_pressed(move |_, key, _, state| {
            if key == gtk::gdk::Key::Return && state.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
                ui2.step(true);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        entry.add_controller(shift_controller);

        let ui2 = ui.clone();
        replace_button.connect_clicked(move |_| ui2.replace_current());
        let ui2 = ui.clone();
        replace_all_button.connect_clicked(move |_| ui2.replace_all());

        let ui2 = ui.clone();
        replace_entry.connect_activate(move |_| ui2.replace_current());

        ui
    }

    /// Point the search machinery at a (possibly different) document.
    pub fn attach(&self, buffer: &sourceview::Buffer, view: &sourceview::View) {
        *self.view.borrow_mut() = Some(view.clone());
        let context = sourceview::SearchContext::new(buffer, Some(self.settings.as_ref()));
        context.set_highlight(true);
        *self.context.borrow_mut() = Some(context);
        let text = self.entry.text();
        if !text.is_empty() {
            self.settings.set_search_text(Some(text.as_str()));
            self.step(false);
        }
    }

    /// Show the bar; optionally with the replace row. Prefills the entry
    /// from the current selection, like gedit.
    pub fn open(&self, with_replace: bool) {
        self.bar.set_search_mode(true);
        self.replace_row.set_visible(with_replace);
        if self.entry.text().is_empty() {
            if let Some(context) = self.context.borrow().as_ref() {
                let buffer = context.buffer();
                if let Some((start, end)) = buffer.selection_bounds() {
                    if start != end {
                        let text = buffer.text(&start, &end, true).to_string();
                        if text.len() < 100 && !text.contains('\n') {
                            self.entry.set_text(&text);
                        }
                    }
                }
            }
        }
        self.entry.grab_focus();
    }

    /// Jump to next (or previous) match from the selection end.
    fn step(&self, backwards: bool) {
        let context = self.context.borrow().clone();
        let (Some(context), Some(view)) = (context, self.view.borrow().clone()) else {
            return;
        };
        let buffer = context.buffer();
        let from = match buffer.selection_bounds() {
            Some((s, e)) => {
                if backwards { s } else { e }
            }
            None => buffer.start_iter(),
        };
        let found = if backwards {
            context.backward(&from)
        } else {
            context.forward(&from)
        };
        if let Some((m_start, mut m_end, _)) = found {
            buffer.select_range(&m_start, &m_end);
            view.scroll_to_iter(&mut m_end, 0.1, false, 0.5, 0.5);
        }
    }

    /// Replace the current match (the selection must be one).
    fn replace_current(&self) {
        let context = self.context.borrow().clone();
        let Some(context) = context else { return };
        let buffer = context.buffer();
        let Some((start, end)) = buffer.selection_bounds() else {
            return;
        };
        if start == end {
            return;
        }
        let replacement = self.replace_entry.text().to_string();
        let mut m_start = start;
        let mut m_end = end;
        if context
            .replace(&mut m_start, &mut m_end, replacement.as_str())
            .is_ok()
        {
            buffer.select_range(&m_end, &m_end);
            self.step(false);
        }
    }

    fn replace_all(&self) {
        let context = self.context.borrow().clone();
        let Some(context) = context else { return };
        let replacement = self.replace_entry.text().to_string();
        let _ = context.replace_all(replacement.as_str());
    }
}
