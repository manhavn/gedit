//! gedit-rs: a gedit-like text editor for Linux.
//!
//! GTK4 + GtkSourceView. Single window, tabbed documents, syntax
//! highlighting, search & replace, goto-line, dark mode, preferences.

mod document;
mod prefs;
mod search;
mod utils;

use document::Document;
use gtk::prelude::*;
use gtk::{gio, glib};
use prefs::Prefs;
use search::SearchUi;
use sourceview::prelude::*;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

const APP_ID: &str = "com.geditrust.Gedit";

struct DocEntry {
    doc: Rc<RefCell<Document>>,
    label: gtk::Label,
    dot: gtk::Label,
}

struct Win {
    window: gtk::ApplicationWindow,
    notebook: gtk::Notebook,
    docs: RefCell<Vec<DocEntry>>,
    prefs: Rc<RefCell<Prefs>>,
    /// View-menu "Dark Mode" toggle; kept as a field so the Preferences
    /// dialog can keep its state in sync (and vice versa).
    dark_action: gio::SimpleAction,
    search: Rc<SearchUi>,
    #[allow(deprecated)] // ComboBoxText is deprecated since GTK 4.10 but simplest here.
    lang_combo: gtk::ComboBoxText,
    pos_label: gtk::Label,
    statusbar: gtk::Box,
    untitled_count: RefCell<u32>,
}

thread_local! {
    static WIN: RefCell<Option<Rc<Win>>> = const { RefCell::new(None) };
    static SNAPSHOT_PATH: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

fn main() {
    // Must run before GTK resolves any theme CSS (see fn docs).
    register_theme_gresource();

    // `--chrome-snapshot <file.png>`: render the window chrome (header bar +
    // content + status bar) to a PNG and exit. Used for visual verification
    // since external screenshot tools are blocked by this Wayland session.
    let raw_args: Vec<String> = std::env::args().collect();
    let mut args: Vec<String> = Vec::with_capacity(raw_args.len());
    let mut i = 0;
    while i < raw_args.len() {
        if raw_args[i] == "--chrome-snapshot" && i + 1 < raw_args.len() {
            SNAPSHOT_PATH.with(|p| *p.borrow_mut() = Some(PathBuf::from(&raw_args[i + 1])));
            i += 2;
        } else {
            args.push(raw_args[i].clone());
            i += 1;
        }
    }

    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    // Record the final tab layout on a clean exit; add/remove already keep
    // the cache fresh, this captures the active tab.
    app.connect_shutdown(|_| {
        WIN.with(|cell| {
            if let Some(win) = cell.borrow().as_ref() {
                win.save_session();
            }
        });
    });

    app.connect_activate(|app| {
        let win = ensure_window(app);
        win.window.present();
        schedule_chrome_snapshot(&win);
    });

    app.connect_open(|app, files, _| {
        let win = ensure_window(app);
        for file in files {
            if let Some(path) = file.path() {
                win.open_path(path);
            }
        }
        win.window.present();
        schedule_chrome_snapshot(&win);
    });

    app.run_with_args(&args);
}

/// `--chrome-snapshot <file.png>` half of the flag parsed in `main`: render
/// the window chrome once layout settles, then exit. Shared by `activate`
/// (no file arguments) and `open` (file arguments), so the flag works either way.
fn schedule_chrome_snapshot(win: &Rc<Win>) {
    let Some(path) = SNAPSHOT_PATH.with(|p| p.borrow().clone()) else {
        return;
    };
    let win = win.clone();
    glib::spawn_future_local(async move {
        // Let the theme and layout settle first.
        glib::timeout_future(std::time::Duration::from_millis(500)).await;
        match render_chrome(&win.window, &path) {
            Ok(()) => std::process::exit(0),
            Err(e) => {
                eprintln!("chrome-snapshot failed: {e}");
                std::process::exit(1);
            }
        }
    });
}

fn ensure_window(app: &gtk::Application) -> Rc<Win> {
    WIN.with(|cell| {
        if let Some(win) = cell.borrow().as_ref() {
            return win.clone();
        }
        let win = Win::new(app);
        *cell.borrow_mut() = Some(win.clone());
        win
    })
}

impl Win {
    #[allow(deprecated)] // ComboBoxText is deprecated since GTK 4.10 but simplest for a static list.
    fn new(app: &gtk::Application) -> Rc<Self> {
        let prefs = Rc::new(RefCell::new(Prefs::load()));
        prefs.borrow().apply_color_scheme();

        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Untitled Document")
            .default_width(1000)
            .default_height(700)
            .build();

        let notebook = gtk::Notebook::new();
        notebook.set_scrollable(true);
        notebook.set_group_name(Some("documents"));
        notebook.set_hexpand(true);
        notebook.set_vexpand(true);

        let search = SearchUi::new();

        // --- Status bar -------------------------------------------------
        let lang_combo = gtk::ComboBoxText::new();
        lang_combo.append(Some(""), "Plain Text");
        let manager = sourceview::LanguageManager::default();
        for id in manager.language_ids() {
            if let Some(lang) = manager.language(&id) {
                lang_combo.append(Some(id.as_str()), lang.name().as_str());
            }
        }
        lang_combo.set_active(Some(0));

        let pos_label = gtk::Label::new(Some("Line 1, Column 1"));
        let pos_button = gtk::MenuButton::new();
        pos_button.set_child(Some(&pos_label));
        pos_button.set_tooltip_text(Some("Go to line"));
        let goto_spin = gtk::SpinButton::with_range(1.0, 1_000_000.0, 1.0);
        let goto_button = gtk::Button::with_label("Go");
        goto_button.add_css_class("suggested-action");
        let goto_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        goto_row.append(&goto_spin);
        goto_row.append(&goto_button);
        goto_row.set_margin_top(6);
        goto_row.set_margin_bottom(6);
        goto_row.set_margin_start(6);
        goto_row.set_margin_end(6);
        let goto_popover = gtk::Popover::new();
        goto_popover.set_child(Some(&goto_row));
        pos_button.set_popover(Some(&goto_popover));

        let statusbar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        statusbar.append(&lang_combo);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        statusbar.append(&spacer);
        statusbar.append(&pos_button);
        statusbar.set_margin_top(2);
        statusbar.set_margin_bottom(2);
        statusbar.set_margin_start(6);
        statusbar.set_margin_end(6);
        statusbar.set_visible(prefs.borrow().show_bottom_bar);

        // --- Header bar --------------------------------------------------
        let header = gtk::HeaderBar::new();

        let btn_new = icon_button("document-new-symbolic", "New document");
        let btn_open = icon_button("document-open-symbolic", "Open file");
        let btn_save = icon_button("document-save-symbolic", "Save");
        let btn_undo = icon_button("edit-undo-symbolic", "Undo");
        let btn_redo = icon_button("edit-redo-symbolic", "Redo");
        let btn_search = icon_button("edit-find-symbolic", "Find and replace");

        header.pack_start(&btn_new);
        header.pack_start(&btn_open);
        header.pack_start(&btn_save);
        header.pack_start(&separator());
        header.pack_start(&btn_undo);
        header.pack_start(&btn_redo);
        header.pack_start(&separator());
        header.pack_start(&btn_search);

        let menu_model = build_menu();
        let menu_button = gtk::MenuButton::new();
        menu_button.set_icon_name("open-menu-symbolic");
        menu_button.set_tooltip_text(Some("Main menu"));
        menu_button.set_menu_model(Some(&menu_model));
        header.pack_end(&menu_button);

        // --- Layout ------------------------------------------------------
        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
        vbox.append(&search.bar);
        vbox.append(&notebook);
        vbox.append(&statusbar);
        window.set_titlebar(Some(&header));
        window.set_child(Some(&vbox));

        let dark_action = gio::SimpleAction::new_stateful(
            "dark-mode",
            None,
            &glib::Variant::from(prefs.borrow().dark_mode),
        );

        let win = Rc::new(Self {
            window,
            notebook,
            docs: RefCell::new(Vec::new()),
            prefs,
            dark_action,
            search,
            lang_combo,
            pos_label,
            statusbar,
            untitled_count: RefCell::new(0),
        });

        win.setup_actions(
            app,
            &btn_new,
            &btn_open,
            &btn_save,
            &btn_undo,
            &btn_redo,
            &btn_search,
        );
        win.setup_notebook_signals(&goto_spin, &goto_button, &goto_popover);
        win.setup_language_combo();

        // The search bar grabs typing while open; keep it above the notebook.
        win.search.bar.set_key_capture_widget(Some(&win.notebook));

        // Restore the previous run's tabs (Sublime-style session cache);
        // a fresh session starts with one empty document, like gedit.
        if !win.restore_session() {
            win.new_document();
        }
        win
    }

    #[allow(deprecated)] // ComboBoxText is deprecated since 4.10.
    fn setup_language_combo(self: &Rc<Self>) {
        let win = self.clone();
        self.lang_combo.connect_changed(move |combo| {
            let Some(id) = combo.active_id() else { return };
            let Some(doc) = win.current_doc() else { return };
            let manager = sourceview::LanguageManager::default();
            let lang = if id.is_empty() {
                None
            } else {
                manager.language(&id)
            };
            doc.borrow().buffer.set_language(lang.as_ref());
        });
    }

    fn setup_actions(
        self: &Rc<Self>,
        app: &gtk::Application,
        btn_new: &gtk::Button,
        btn_open: &gtk::Button,
        btn_save: &gtk::Button,
        btn_undo: &gtk::Button,
        btn_redo: &gtk::Button,
        btn_search: &gtk::Button,
    ) {
        let group = gio::SimpleActionGroup::new();

        let win = self.clone();
        add_action(&group, "new", move |_| win.new_document());
        let win = self.clone();
        add_action(&group, "open", move |_| win.open_dialog());
        let win = self.clone();
        add_action(&group, "save", move |_| win.save_current());
        let win = self.clone();
        add_action(&group, "save-as", move |_| win.save_current_as());
        let win = self.clone();
        add_action(&group, "undo", move |_| {
            if let Some(doc) = win.current_doc() {
                doc.borrow().buffer.undo();
            }
        });
        let win = self.clone();
        add_action(&group, "redo", move |_| {
            if let Some(doc) = win.current_doc() {
                doc.borrow().buffer.redo();
            }
        });
        let win = self.clone();
        add_action(&group, "find", move |_| win.search.toggle(false));
        let win = self.clone();
        add_action(&group, "replace", move |_| win.search.toggle(true));
        let win = self.clone();
        add_action(&group, "close-tab", move |_| win.close_current_tab());

        // Stateful: word wrap
        let wrap_action = gio::SimpleAction::new_stateful(
            "word-wrap",
            None,
            &glib::Variant::from(self.prefs.borrow().word_wrap),
        );
        let win = self.clone();
        wrap_action.connect_activate(move |action, _| {
            let current = action.state().and_then(|v| v.get::<bool>()).unwrap_or(false);
            let new_state = !current;
            action.set_state(&glib::Variant::from(new_state));
            {
                let mut p = win.prefs.borrow_mut();
                p.word_wrap = new_state;
                p.save();
            }
            let prefs = *win.prefs.borrow();
            for entry in win.docs.borrow().iter() {
                entry.doc.borrow().apply_prefs(&prefs);
            }
        });
        group.add_action(&wrap_action);

        // Stateful: dark mode (created in `Win::new` so Preferences can sync it)
        let win = self.clone();
        let dark_action = self.dark_action.clone();
        dark_action.connect_activate(move |action, _| {
            let new_state = !win.prefs.borrow().dark_mode;
            action.set_state(&glib::Variant::from(new_state));
            {
                let mut p = win.prefs.borrow_mut();
                p.dark_mode = new_state;
                p.save();
            }
            win.apply_prefs();
        });
        group.add_action(&dark_action);

        let bottom_bar_action = gio::SimpleAction::new_stateful(
            "bottom-bar",
            None,
            &glib::Variant::from(self.prefs.borrow().show_bottom_bar),
        );
        let win = self.clone();
        bottom_bar_action.connect_activate(move |action, _| {
            let visible = !win.prefs.borrow().show_bottom_bar;
            action.set_state(&glib::Variant::from(visible));
            win.statusbar.set_visible(visible);
            let mut prefs = win.prefs.borrow_mut();
            prefs.show_bottom_bar = visible;
            prefs.save();
        });
        group.add_action(&bottom_bar_action);

        let win = self.clone();
        add_action(&group, "preferences", move |_| win.show_preferences());
        let win = self.clone();
        add_action(&group, "about", move |_| win.show_about());

        self.window.insert_action_group("win", Some(&group));

        let quit = gio::SimpleAction::new("quit", None);
        let win = self.clone();
        quit.connect_activate(move |_, _| win.window.close());
        app.add_action(&quit);
        app.set_accels_for_action("app.quit", &["<Control>q"]);

        let accels: &[(&str, &[&str])] = &[
            ("win.new", &["<Control>n"]),
            ("win.open", &["<Control>o"]),
            ("win.save", &["<Control>s"]),
            ("win.save-as", &["<Control><Shift>s"]),
            ("win.undo", &["<Control>z"]),
            ("win.redo", &["<Control><Shift>z", "<Control>y"]),
            ("win.find", &["<Control>f"]),
            ("win.replace", &["<Control>h"]),
            ("win.close-tab", &["<Control>w"]),
        ];
        for (action, keys) in accels {
            app.set_accels_for_action(action, keys);
        }

        // Wire header buttons to the same actions.
        bind_button(btn_new, &group, "new");
        bind_button(btn_open, &group, "open");
        bind_button(btn_save, &group, "save");
        bind_button(btn_undo, &group, "undo");
        bind_button(btn_redo, &group, "redo");
        let win = self.clone();
        btn_search.connect_clicked(move |_| win.search.toggle(true));
    }

    fn setup_notebook_signals(
        self: &Rc<Self>,
        goto_spin: &gtk::SpinButton,
        goto_button: &gtk::Button,
        goto_popover: &gtk::Popover,
    ) {
        // Tab switch: refresh title, search context, language, cursor info.
        let win = self.clone();
        self.notebook.connect_switch_page(move |_, _, page| {
            win.on_tab_switch(page as usize);
        });

        // Goto-line: button or Enter in the spin.
        let win = self.clone();
        let popover2 = goto_popover.clone();
        let spin2 = goto_spin.clone();
        goto_button.connect_clicked(move |_| {
            win.goto_line(spin2.value_as_int());
            popover2.popdown();
        });
        let win = self.clone();
        let popover2 = goto_popover.clone();
        goto_spin.connect_activate(move |spin| {
            win.goto_line(spin.value_as_int());
            popover2.popdown();
        });
        // Prefill the spin with the cursor line when the popover opens.
        let win = self.clone();
        let spin2 = goto_spin.clone();
        goto_popover.connect_show(move |_| {
            if let Some(doc) = win.current_doc() {
                let line = cursor_iter(&doc.borrow().buffer).line() + 1;
                spin2.set_value(line as f64);
            }
        });
    }

    // --- Document management -------------------------------------------

    fn current_page(&self) -> Option<usize> {
        self.notebook.current_page().map(|p| p as usize)
    }

    fn current_doc(&self) -> Option<Rc<RefCell<Document>>> {
        let idx = self.current_page()?;
        self.docs.borrow().get(idx).map(|e| e.doc.clone())
    }

    fn new_document(self: &Rc<Self>) {
        let n = {
            let mut count = self.untitled_count.borrow_mut();
            *count += 1;
            *count
        };
        let doc = Document::new(&self.prefs.borrow(), &format!("Untitled Document {n}"));
        doc.borrow().buffer.set_modified(false);
        self.add_doc(doc);
    }

    fn open_path(self: &Rc<Self>, path: PathBuf) {
        // Canonical, so a command-line file that is also in the session
        // cache maps onto one tab instead of opening twice.
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        let already = self
            .docs
            .borrow()
            .iter()
            .position(|e| e.doc.borrow().file.as_deref() == Some(path.as_path()));
        if let Some(idx) = already {
            self.notebook.set_current_page(Some(idx as u32));
            self.on_tab_switch(idx);
            return;
        }
        let doc = Document::new(&self.prefs.borrow(), "Untitled Document");
        // Read the document's name only after `load_file`, which is what sets
        // `file` — naming before it yields the untitled placeholder instead.
        let result = doc.borrow_mut().load_file(&path);
        match result {
            Ok(()) => self.add_doc(doc),
            Err(e) => utils::show_error(&self.window, &e),
        }
    }

    fn add_doc(self: &Rc<Self>, doc: Rc<RefCell<Document>>) {
        // Tab label: file name + close button; hover reveals the full path,
        // since the label itself is ellipsized.
        let (display, full) = {
            let doc = doc.borrow();
            (doc.display_name(), doc.full_name())
        };
        let label = gtk::Label::new(Some(&display));
        label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        label.set_width_chars(12);
        label.set_max_width_chars(24);
        label.set_tooltip_text(Some(&full));

        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.add_css_class("flat");
        close.add_css_class("circular");
        close.set_tooltip_text(Some("Close document"));
        let tab_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        tab_box.append(&label);
        tab_box.append(&close);

        // Unsaved-changes dot, centered on the tab's bottom edge. Driven by
        // opacity (not visibility) so the tab keeps a constant size. The
        // bullet is scaled down to ~6px: full-size ● reads as a marker
        // light, a plain • renders down to ~3px and is nearly invisible.
        let dot = gtk::Label::new(Some("●"));
        let dot_attrs = gtk::pango::AttrList::new();
        dot_attrs.insert(gtk::pango::AttrFloat::new_scale(0.55));
        dot.set_attributes(Some(&dot_attrs));
        dot.set_halign(gtk::Align::Center);
        dot.set_valign(gtk::Align::End);
        dot.set_tooltip_text(Some("Unsaved changes"));
        let tab = gtk::Overlay::new();
        tab.set_child(Some(&tab_box));
        tab.add_overlay(&dot);

        let idx = self.docs.borrow().len();
        // Page = the view inside a ScrolledWindow. A raw GtkTextView reports
        // a minimum height equal to its entire buffer, which pinned the
        // window's minimum height to the file length — impossible to shrink
        // the window, and nothing ever scrolled.
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_child(Some(&doc.borrow().view));
        scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
        scroll.set_hexpand(true);
        scroll.set_vexpand(true);
        self.notebook.append_page(&scroll, Some(&tab));
        self.docs.borrow_mut().push(DocEntry {
            doc: doc.clone(),
            label: label.clone(),
            dot: dot.clone(),
        });
        // Pick up a pre-existing modified state (restored session tabs).
        self.refresh_modified_ui(&doc);

        // Close button (resolve the page at click time: indices shift).
        let win = self.clone();
        let scroll2 = scroll.clone();
        close.connect_clicked(move |_| {
            let page = win.notebook.page_num(&scroll2);
            if let Some(page) = page {
                win.close_tab(page as usize);
            }
        });

        // Modified flag -> bold tab label, unsaved dot, window title.
        // GtkTextBuffer emits no notify for its "modified" property (GTK
        // 4.24, verified with a catch-all notify listener), so text changes
        // drive the refresh via `changed`; saves call refresh_modified_ui
        // directly after mark_saved.
        let win_weak = Rc::downgrade(self);
        let doc_weak = Rc::downgrade(&doc);
        doc.borrow().buffer.connect_changed(move |_| {
            let (Some(win), Some(doc)) = (win_weak.upgrade(), doc_weak.upgrade()) else {
                return;
            };
            win.refresh_modified_ui(&doc);
        });

        // Cursor position -> status bar.
        let win_weak = Rc::downgrade(self);
        let doc_weak = Rc::downgrade(&doc);
        doc.borrow()
            .buffer
            .connect_notify_local(Some("cursor-position"), move |buffer, _| {
                let (Some(win), Some(doc)) = (win_weak.upgrade(), doc_weak.upgrade()) else {
                    return;
                };
                // Only refresh if this is the visible document.
                if win.current_doc().is_some_and(|d| Rc::ptr_eq(&d, &doc)) {
                    let iter = cursor_iter(buffer);
                    win.pos_label.set_text(&format!(
                        "Line {}, Column {}",
                        iter.line() + 1,
                        iter.line_offset() + 1
                    ));
                }
            });

        self.notebook.set_current_page(Some(idx as u32));
        self.on_tab_switch(idx);
        self.save_session();
    }

    #[allow(deprecated)] // ComboBoxText is deprecated since GTK 4.10.
    fn on_tab_switch(self: &Rc<Self>, idx: usize) {
        let Some(entry) = self.docs.borrow().get(idx).map(|e| e.doc.clone()) else {
            return;
        };
        {
            let doc = entry.borrow();
            self.search.attach(&doc.buffer, &doc.view);
            // Sync the language combo without redundant work.
            let lang_id = doc
                .buffer
                .language()
                .map(|l| l.id().to_string())
                .unwrap_or_default();
            self.lang_combo.set_active_id(Some(&lang_id));
        }
        self.update_title();
        if let Some(doc) = self.current_doc() {
            let iter = cursor_iter(&doc.borrow().buffer);
            self.pos_label.set_text(&format!(
                "Line {}, Column {}",
                iter.line() + 1,
                iter.line_offset() + 1
            ));
        }
    }

    /// Sync the tab (bold label, unsaved dot) and the window title with the
    /// document's modified flag.
    fn refresh_modified_ui(self: &Rc<Self>, doc: &Rc<RefCell<Document>>) {
        let modified = doc.borrow().buffer.is_modified();
        for entry in self.docs.borrow().iter() {
            if Rc::ptr_eq(&entry.doc, doc) {
                if modified {
                    entry.label.add_css_class("heading");
                } else {
                    entry.label.remove_css_class("heading");
                }
                entry.dot.set_opacity(if modified { 1.0 } else { 0.0 });
                break;
            }
        }
        self.update_title();
    }

    fn update_title(self: &Rc<Self>) {
        let Some(doc) = self.current_doc() else { return };
        let doc = doc.borrow();
        let modified = doc.buffer.is_modified();
        let title = if modified {
            format!("{} — gedit-rs (*)", doc.display_name())
        } else {
            format!("{} — gedit-rs", doc.display_name())
        };
        self.window.set_title(Some(&title));
    }

    // --- Saving ---------------------------------------------------------

    /// Ensure the document has a path (asking Save As if needed); returns
    /// false when the user cancelled.
    async fn ensure_saved(self: &Rc<Self>, doc: &Rc<RefCell<Document>>) -> bool {
        let Some(path) = doc.borrow().file.clone() else {
            return self.save_as_flow(doc).await;
        };
        match self.save_doc_to(doc, &path).await {
            Ok(()) => true,
            Err(e) => {
                utils::show_error(&self.window, &e);
                false
            }
        }
    }

    /// Write the document's text to `path` and remember the path. When the
    /// file isn't writable by the current user ("permission denied"), retry
    /// through the system PolicyKit authentication dialog (`pkexec`).
    async fn save_doc_to(
        self: &Rc<Self>,
        doc: &Rc<RefCell<Document>>,
        path: &std::path::Path,
    ) -> Result<(), String> {
        let text = doc.borrow().text();
        match std::fs::write(path, text.as_bytes()) {
            Ok(()) => {
                doc.borrow_mut().mark_saved(path);
                self.refresh_modified_ui(doc);
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                utils::elevated_write(path, &text).await?;
                doc.borrow_mut().mark_saved(path);
                self.refresh_modified_ui(doc);
                Ok(())
            }
            Err(e) => Err(format!("{}: {}", path.display(), e)),
        }
    }

    async fn save_as_flow(self: &Rc<Self>, doc: &Rc<RefCell<Document>>) -> bool {
        let dialog = gtk::FileDialog::new();
        dialog.set_title("Save As");
        if let Some(path) = doc.borrow().file.clone() {
            dialog.set_initial_file(Some(&gio::File::for_path(path)));
        }
        match dialog.save_future(Some(&self.window)).await {
            Ok(file) => {
                if let Some(path) = file.path() {
                    match self.save_doc_to(doc, &path).await {
                        Ok(()) => {
                            self.refresh_label(doc);
                            true
                        }
                        Err(e) => {
                            utils::show_error(&self.window, &e);
                            false
                        }
                    }
                } else {
                    false
                }
            }
            Err(_) => false, // cancelled
        }
    }

    fn save_current(self: &Rc<Self>) {
        let Some(doc) = self.current_doc() else { return };
        let win = self.clone();
        glib::spawn_future_local(async move {
            win.ensure_saved(&doc).await;
        });
    }

    fn save_current_as(self: &Rc<Self>) {
        let Some(doc) = self.current_doc() else { return };
        let win = self.clone();
        glib::spawn_future_local(async move {
            win.save_as_flow(&doc).await;
        });
    }

    /// Ask "Save changes to X before closing?" when modified.
    /// Returns true when it is OK to discard/close.
    async fn confirm_save(self: &Rc<Self>, doc: &Rc<RefCell<Document>>) -> bool {
        if !doc.borrow().buffer.is_modified() {
            return true;
        }
        let name = doc.borrow().full_name();
        let cancel = gtk::ResponseType::Cancel;
        let discard = gtk::ResponseType::No;
        let save = gtk::ResponseType::Yes;
        let response = utils::ask(
            &self.window,
            &format!("Save changes to document “{name}” before closing?"),
            Some("If you don't save, changes will be permanently lost."),
            &[("Cancel", cancel), ("Discard", discard), ("Save", save)],
        )
        .await;
        match response {
            r if r == save => self.ensure_saved(doc).await,
            r if r == discard => true,
            _ => false,
        }
    }

    /// Re-sync the tab label, its hover tooltip, and the window title after
    /// the document's name changes (Save As assigns the path).
    fn refresh_label(self: &Rc<Self>, doc: &Rc<RefCell<Document>>) {
        for entry in self.docs.borrow().iter() {
            if Rc::ptr_eq(&entry.doc, doc) {
                let doc = doc.borrow();
                entry.label.set_text(&doc.display_name());
                entry.label.set_tooltip_text(Some(&doc.full_name()));
                break;
            }
        }
        self.update_title();
    }

    // --- Tabs -----------------------------------------------------------

    fn close_current_tab(self: &Rc<Self>) {
        let Some(idx) = self.current_page() else { return };
        self.close_tab(idx);
    }

    fn close_tab(self: &Rc<Self>, idx: usize) {
        let doc = match self.docs.borrow().get(idx) {
            Some(e) => e.doc.clone(),
            None => return,
        };
        let win = self.clone();
        glib::spawn_future_local(async move {
            if win.confirm_save(&doc).await {
                win.remove_doc(idx);
            }
        });
    }

    fn remove_doc(self: &Rc<Self>, idx: usize) {
        self.docs.borrow_mut().remove(idx);
        self.notebook.remove_page(Some(idx as u32));
        if self.docs.borrow().is_empty() {
            // gedit closes the window with the last tab.
            self.window.close();
            return;
        }
        let new_idx = idx.min(self.docs.borrow().len() - 1);
        self.notebook.set_current_page(Some(new_idx as u32));
        self.on_tab_switch(new_idx);
        self.save_session();
    }

    // --- Session cache ---------------------------------------------------

    /// Persist the open tabs to the per-session cache (Sublime-style) so
    /// the next launch of this Ubuntu session reopens them. Best-effort: a
    /// missing or full /tmp must never break adding a tab or quitting.
    fn save_session(&self) {
        let file = session_file();
        if let Some(dir) = file.parent()
            && std::fs::create_dir_all(dir).is_err()
        {
            return;
        }
        let kf = glib::KeyFile::new();
        let docs = self.docs.borrow();
        let _ = kf.set_integer("session", "count", docs.len() as i32);
        let _ = kf.set_integer("session", "active", self.current_page().unwrap_or(0) as i32);
        for (i, entry) in docs.iter().enumerate() {
            let group = format!("tab-{i}");
            let doc = entry.doc.borrow();
            if let Some(p) = &doc.file {
                let _ = kf.set_string(&group, "path", &p.to_string_lossy());
            }
            let _ = kf.set_boolean(&group, "modified", doc.buffer.is_modified());
            let _ = kf.set_string(&group, "content", &doc.text());
        }
        let _ = kf.save_to_file(&file);
    }

    /// Reopen the tabs recorded by the previous run. Untouched files are
    /// reloaded from disk (external edits win); modified ones — and files
    /// that vanished since — come back from the cached copy, flagged
    /// unsaved. Returns true when at least one tab was restored.
    fn restore_session(self: &Rc<Self>) -> bool {
        let kf = glib::KeyFile::new();
        if kf
            .load_from_file(session_file(), glib::KeyFileFlags::NONE)
            .is_err()
        {
            return false;
        }
        let count = kf.integer("session", "count").unwrap_or(0);
        let active = kf.integer("session", "active").unwrap_or(0);
        let mut restored = 0usize;
        for i in 0..count {
            let group = format!("tab-{i}");
            let content = kf
                .string(&group, "content")
                .map(|s| s.to_string())
                .unwrap_or_default();
            let modified = kf.boolean(&group, "modified").unwrap_or(false);
            let path = kf.string(&group, "path").ok().map(PathBuf::from);
            let doc = match &path {
                Some(p) => {
                    let d = Document::new(&self.prefs.borrow(), "Untitled Document");
                    if !modified && d.borrow_mut().load_file(p).is_ok() {
                        d
                    } else {
                        d.borrow_mut().restore(Some(p), &content, true);
                        d
                    }
                }
                None => {
                    let n = {
                        let mut counter = self.untitled_count.borrow_mut();
                        *counter += 1;
                        *counter
                    };
                    let d = Document::new(&self.prefs.borrow(), &format!("Untitled Document {n}"));
                    d.borrow_mut().restore(None, &content, modified);
                    d
                }
            };
            self.add_doc(doc);
            restored += 1;
        }
        if restored == 0 {
            return false;
        }
        let last = self.docs.borrow().len();
        let idx = (active as usize).min(last - 1);
        self.notebook.set_current_page(Some(idx as u32));
        self.on_tab_switch(idx);
        true
    }

    // --- Goto line ------------------------------------------------------

    fn goto_line(self: &Rc<Self>, line: i32) -> bool {
        let Some(doc) = self.current_doc() else { return false };
        let doc = doc.borrow();
        let line = line.clamp(1, doc.buffer.line_count());
        let Some(mut iter) = doc.buffer.iter_at_line(line - 1) else {
            return false;
        };
        doc.buffer.select_range(&iter, &iter);
        doc.view.scroll_to_iter(&mut iter, 0.1, false, 0.5, 0.5);
        self.pos_label
            .set_text(&format!("Line {}, Column {}", line, 1));
        true
    }

    // --- Open dialog ----------------------------------------------------

    fn open_dialog(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::new();
        dialog.set_title("Open File");
        let win = self.clone();
        glib::spawn_future_local(async move {
            match dialog.open_multiple_future(Some(&win.window)).await {
                Ok(files) => {
                    for i in 0..files.n_items() {
                        if let Some(file) = files.item(i).and_downcast::<gio::File>() {
                            if let Some(path) = file.path() {
                                win.open_path(path);
                            }
                        }
                    }
                }
                Err(_) => {} // cancelled
            }
        });
    }

    // --- Preferences / About -------------------------------------------

    /// Push the current preferences to GTK (dark/light) and every open
    /// document (view options, editor style scheme).
    fn apply_prefs(self: &Rc<Self>) {
        let prefs = *self.prefs.borrow();
        prefs.apply_color_scheme();
        for entry in self.docs.borrow().iter() {
            entry.doc.borrow().apply_prefs(&prefs);
        }
    }

    fn show_preferences(self: &Rc<Self>) {
        let dialog = gtk::Window::builder()
            .transient_for(&self.window)
            .modal(true)
            .title("Preferences")
            .default_width(360)
            .build();

        let grid = gtk::Grid::new();
        grid.set_row_spacing(12);
        grid.set_column_spacing(12);
        grid.set_margin_top(12);
        grid.set_margin_bottom(12);
        grid.set_margin_start(12);
        grid.set_margin_end(12);

        let toggles: &[(&str, fn(&Prefs) -> bool, fn(&mut Prefs, bool))] = &[
            ("Display line numbers", |p| p.line_numbers, |p, v| p.line_numbers = v),
            (
                "Highlight current line",
                |p| p.highlight_current_line,
                |p, v| p.highlight_current_line = v,
            ),
            ("Enable automatic indentation", |p| p.auto_indent, |p, v| p.auto_indent = v),
            ("Insert spaces instead of tabs", |p| p.insert_spaces, |p, v| p.insert_spaces = v),
            ("Enable word wrap", |p| p.word_wrap, |p, v| p.word_wrap = v),
            ("Use dark color scheme", |p| p.dark_mode, |p, v| p.dark_mode = v),
        ];

        for (i, (title, get, set)) in toggles.iter().enumerate() {
            let check = gtk::CheckButton::with_label(title);
            check.set_active(get(&self.prefs.borrow()));
            let win = self.clone();
            let set = *set;
            check.connect_toggled(move |c| {
                let active = c.is_active();
                {
                    let mut p = win.prefs.borrow_mut();
                    set(&mut p, active);
                    p.save();
                }
                win.apply_prefs();
                // Keep the View-menu checkmark truthful for every toggle;
                // only dark-mode has a mirrored action, others are no-ops.
                win.dark_action
                    .set_state(&glib::Variant::from(win.prefs.borrow().dark_mode));
            });
            grid.attach(&check, 0, i as i32, 1, 1);
        }

        let close = gtk::Button::with_label("Close");
        close.set_halign(gtk::Align::End);
        close.set_margin_end(12);
        close.set_margin_bottom(12);
        let dialog2 = dialog.clone();
        close.connect_clicked(move |_| dialog2.close());

        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 12);
        vbox.append(&grid);
        vbox.append(&close);
        dialog.set_child(Some(&vbox));
        dialog.present();
    }

    fn show_about(self: &Rc<Self>) {
        let about = gtk::AboutDialog::builder()
            .transient_for(&self.window)
            .modal(true)
            .program_name("gedit-rs")
            .version("0.1.0")
            .comments(
                "A gedit-like text editor for Linux, written in Rust with GTK4 and GtkSourceView.",
            )
            .license_type(gtk::License::MitX11)
            .authors(["gedit-rs contributors"])
            .build();
        about.present();
    }
}

// --- free helpers -------------------------------------------------------

fn build_menu() -> gio::Menu {
    let menu = gio::Menu::new();

    let file = gio::Menu::new();
    file.append(Some("New"), Some("win.new"));
    file.append(Some("Open…"), Some("win.open"));
    file.append(Some("Save"), Some("win.save"));
    file.append(Some("Save As…"), Some("win.save-as"));
    file.append(Some("Close Tab"), Some("win.close-tab"));
    menu.append_section(None, &file);

    let edit = gio::Menu::new();
    edit.append(Some("Undo"), Some("win.undo"));
    edit.append(Some("Redo"), Some("win.redo"));
    edit.append(Some("Find…"), Some("win.find"));
    edit.append(Some("Replace…"), Some("win.replace"));
    menu.append_section(None, &edit);

    let view = gio::Menu::new();
    view.append_item(&gio::MenuItem::new(Some("Word Wrap"), Some("win.word-wrap")));
    view.append_item(&gio::MenuItem::new(Some("Dark Mode"), Some("win.dark-mode")));
    view.append_item(&gio::MenuItem::new(Some("Bottom Bar"), Some("win.bottom-bar")));
    menu.append_section(None, &view);

    let app_menu = gio::Menu::new();
    app_menu.append(Some("Preferences"), Some("win.preferences"));
    app_menu.append(Some("About gedit-rs"), Some("win.about"));
    app_menu.append(Some("Quit"), Some("app.quit"));
    menu.append_section(None, &app_menu);

    menu
}

/// Cursor position via the `cursor-position` buffer property
/// (`iter_at_cursor` is not bound by gtk4-rs).
fn cursor_iter(buffer: &sourceview::Buffer) -> gtk::TextIter {
    let offset = buffer.property::<i32>("cursor-position");
    buffer.iter_at_offset(offset)
}

/// Register the active GTK theme's `gtk.gresource` with the GLib resource
/// registry.
///
/// Yaru ships its GTK4 CSS inside `gtk.gresource` and the theme's
/// `gtk.css`/`gtk-dark.css` files are only `@import url("resource://…")`
/// wrappers. GTK loads `gtk-dark.css` first under a dark preference but only
/// registers the gresource as a side effect of loading `gtk.css` — so every
/// `resource://` import fails ("resource … does not exist"), the theme rules
/// never apply, and the header bar / tab strip / status bar render fully
/// transparent (visible desktop behind them). Registering up front fixes the
/// ordering. glib-rs ships no binding for this, hence `gio::ffi`.
///
/// The leaked `GResource` handle is intentional: the process-global
/// registry keeps a raw pointer that must stay valid for the app's life.
fn register_theme_gresource() {
    let theme = gio::SettingsSchemaSource::default()
        .and_then(|source| source.lookup("org.gnome.desktop.interface", true))
        .map(|schema| {
            let settings =
                gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None);
            settings.string("gtk-theme").to_string()
        })
        .unwrap_or_else(|| "Yaru".to_string());

    let mut dirs: Vec<std::path::PathBuf> = vec![glib::user_data_dir().join("themes")];
    if let Some(xdg) = std::env::var_os("XDG_DATA_DIRS") {
        dirs.extend(std::env::split_paths(&xdg).map(|d| d.join("themes")));
    }
    dirs.push(std::path::PathBuf::from("/usr/share/themes"));

    let Some(path) = dirs
        .iter()
        .map(|d| d.join(&theme).join("gtk-4.0").join("gtk.gresource"))
        .find(|p| p.exists())
    else {
        return;
    };

    let Ok(data) = std::fs::read(&path) else {
        eprintln!("theme-gresource: cannot read {}", path.display());
        return;
    };
    use glib::translate::ToGlibPtr;
    let bytes = glib::Bytes::from_owned(data);
    unsafe {
        let mut error: *mut glib::ffi::GError = std::ptr::null_mut();
        let data_ptr = ToGlibPtr::<*const glib::ffi::GBytes>::to_glib_none(&bytes).0;
        let resource =
            gio::ffi::g_resource_new_from_data(data_ptr as *mut glib::ffi::GBytes, &mut error);
        if resource.is_null() {
            if !error.is_null() {
                glib::ffi::g_error_free(error);
            }
            eprintln!("theme-gresource: parse failed for {}", path.display());
            return;
        }
        gio::ffi::g_resources_register(resource);
        // `resource` (a raw pointer) is deliberately never unreferenced: the
        // registry must be able to use it for the process lifetime.
        // Keep the backing bytes alive for the same reason.
        std::mem::forget(bytes);
    }
}

/// Render header bar + window body (incl. status bar) stacked into one PNG.
fn render_chrome(window: &gtk::ApplicationWindow, path: &std::path::Path) -> Result<(), String> {
    let width = window.width();
    let header = window.titlebar().ok_or("window has no titlebar")?;
    let body = window.child().ok_or("window has no child")?;
    let header_h = header.height().max(1);
    let body_h = body.height().max(1);
    let total_h = header_h + body_h;

    eprintln!("chrome-snapshot: {width}x{total_h} (header={header_h} body={body_h})");
    {
        let mut child = body.first_child();
        while let Some(c) = child {
            eprintln!(
                "  body child: {} {}x{}",
                c.type_().name(),
                c.width(),
                c.height()
            );
            child = c.next_sibling();
        }
    }

    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width, total_h)
        .map_err(|e| e.to_string())?;
    let cr = cairo::Context::new(&surface).map_err(|e| e.to_string())?;

    // Magenta backdrop: parts of the chrome (plain Boxes, etc.) paint no
    // background of their own and inherit the window background, which a
    // child-only snapshot cannot capture — so magenta here is expected for
    // those, while any *themed* widget showing magenta would indicate a
    // theme problem.
    cr.set_source_rgb(1.0, 0.0, 1.0);
    cr.paint().map_err(|e| e.to_string())?;

    let snap = gtk::Snapshot::new();
    window.snapshot_child(&header, &snap);
    if let Some(node) = snap.to_node() {
        node.draw(&cr);
    }

    // NOTE: `snapshot_child` already positions each child at its allocation
    // offset within the parent, so no manual translation is needed.
    let snap = gtk::Snapshot::new();
    window.snapshot_child(&body, &snap);
    if let Some(node) = snap.to_node() {
        node.draw(&cr);
    }

    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    surface
        .write_to_png(&mut std::io::BufWriter::new(file))
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.set_tooltip_text(Some(tooltip));
    button
}

fn separator() -> gtk::Separator {
    gtk::Separator::new(gtk::Orientation::Vertical)
}

/// Session cache location: a per-uid directory under /tmp, which the OS
/// wipes on reboot — the cache never outlives the Ubuntu session that
/// wrote it.
fn session_file() -> PathBuf {
    use std::os::unix::fs::MetadataExt;
    let uid = std::fs::metadata("/proc/self")
        .map(|m| m.uid())
        .unwrap_or(0);
    std::env::temp_dir()
        .join(format!("gedit-rs-{uid}"))
        .join("session.ini")
}

fn add_action<F>(group: &gio::SimpleActionGroup, name: &str, f: F)
where
    F: Fn(&gio::SimpleAction) + 'static,
{
    let action = gio::SimpleAction::new(name, None);
    action.connect_activate(move |a, _| f(a));
    group.add_action(&action);
}

fn bind_button(button: &gtk::Button, group: &gio::SimpleActionGroup, name: &str) {
    let group = group.clone();
    let name = name.to_string();
    button.connect_clicked(move |_| {
        if let Some(action) = group.lookup_action(&name) {
            action.activate(None);
        }
    });
}
