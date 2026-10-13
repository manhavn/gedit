//! Tiny helpers: awaitable dialog response and blocking error popups.

use gtk::prelude::*;
use gtk::{glib, ResponseType};
use parking_lot::Mutex;
use std::cell::RefCell;
use std::io::Write;
use std::path::Path;
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

struct ResponseState {
    response: Option<ResponseType>,
    waker: Option<Waker>,
}

/// Awaitable `MessageDialog`. Buttons are `(label, response_id)`; the dialog
/// closes itself when a button is pressed.
#[allow(deprecated)] // MessageDialog is the plain-GTK4 way; libadwaita is not linked.
pub fn ask(
    parent: &impl IsA<gtk::Window>,
    text: &str,
    secondary: Option<&str>,
    buttons: &[(&str, ResponseType)],
) -> impl Future<Output = ResponseType> {
    let dialog = gtk::MessageDialog::builder()
        .transient_for(parent)
        .modal(true)
        .buttons(gtk::ButtonsType::None)
        .message_type(gtk::MessageType::Question)
        .text(text)
        .secondary_text(secondary.unwrap_or(""))
        .build();
    for (label, id) in buttons {
        dialog.add_button(label, *id);
    }
    let state = Rc::new(RefCell::new(ResponseState {
        response: None,
        waker: None,
    }));
    let state2 = state.clone();
    dialog.connect_response(move |d, response| {
        state2.borrow_mut().response = Some(response);
        if let Some(w) = state2.borrow_mut().waker.take() {
            w.wake();
        }
        d.close();
    });
    dialog.present();
    DialogFuture { state }
}

pub struct DialogFuture {
    state: Rc<RefCell<ResponseState>>,
}

impl Future for DialogFuture {
    type Output = ResponseType;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<ResponseType> {
        let mut st = self.state.borrow_mut();
        if let Some(r) = st.response.take() {
            Poll::Ready(r)
        } else {
            st.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

/// Write `text` to `path` with root privileges, prompting for the password
/// through the desktop's PolicyKit authentication dialog (`pkexec`). Used by
/// the save flow when a plain write fails with "permission denied".
///
/// The text is piped to `tee`'s standard input, so it never appears on the
/// command line and nothing reaches the disk before authentication succeeds.
/// An existing file keeps its owner and permissions; a newly created one is
/// owned by root with `tee`'s default mode. The wait runs on a background
/// thread so the GTK main loop stays responsive while the dialog is open.
pub fn elevated_write(path: &Path, text: &str) -> impl Future<Output = Result<(), String>> {
    let state = Arc::new(Mutex::new(ElevationState {
        result: None,
        waker: None,
    }));
    let path = path.to_path_buf();
    let text = text.to_string();
    let state2 = state.clone();
    std::thread::spawn(move || {
        let result = pkexec_tee(&path, &text);
        glib::idle_add_once(move || {
            let mut st = state2.lock();
            st.result = Some(result);
            if let Some(w) = st.waker.take() {
                w.wake();
            }
        });
    });
    ElevationFuture { state }
}

struct ElevationState {
    result: Option<Result<(), String>>,
    waker: Option<Waker>,
}

pub struct ElevationFuture {
    state: Arc<Mutex<ElevationState>>,
}

impl Future for ElevationFuture {
    type Output = Result<(), String>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), String>> {
        let mut st = self.state.lock();
        if let Some(r) = st.result.take() {
            Poll::Ready(r)
        } else {
            st.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

/// Blocking half of [`elevated_write`]: run `pkexec tee -- <path>` with `text`
/// on stdin and wait for the (possibly interactive) authentication to finish.
fn pkexec_tee(path: &Path, text: &str) -> Result<(), String> {
    let mut child = Command::new("pkexec")
        .arg("tee")
        .arg("--")
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                "cannot save with elevated privileges: pkexec is not installed".to_string()
            }
            _ => format!("pkexec: {e}"),
        })?;
    let write_result = match child.stdin.take() {
        Some(mut stdin) => stdin.write_all(text.as_bytes()),
        None => Ok(()), // unreachable: stdin is piped
    };
    let output = child
        .wait_with_output()
        .map_err(|e| format!("pkexec: {e}"))?;
    if !output.status.success() {
        // A broken pipe here just means the user dismissed the dialog;
        // the exit status decides the message either way.
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail.trim();
        return Err(if detail.is_empty() {
            format!(
                "authentication failed or was cancelled; “{}” was not saved",
                path.display()
            )
        } else {
            format!("{}: {}", path.display(), detail)
        });
    }
    write_result.map_err(|e| format!("{}: {}", path.display(), e))?;
    Ok(())
}

/// Fire-and-forget error popup.
#[allow(deprecated)]
pub fn show_error(parent: &impl IsA<gtk::Window>, text: &str) {
    let dialog = gtk::MessageDialog::builder()
        .transient_for(parent)
        .modal(true)
        .buttons(gtk::ButtonsType::Close)
        .message_type(gtk::MessageType::Error)
        .text(text)
        .build();
    dialog.connect_response(|d, _| d.close());
    dialog.present();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn write_fake_pkexec(dir: &Path, script: &str) {
        let bin = dir.join("pkexec");
        std::fs::write(&bin, script).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Runs `f` with `dir` prepended to PATH, so the shim shadows the real
    /// `pkexec` while `tee` and everything else stay reachable.
    fn with_path_prepend<T>(dir: &Path, f: impl FnOnce() -> T) -> T {
        let old = std::env::var("PATH").unwrap_or_default();
        unsafe { std::env::set_var("PATH", format!("{}:{}", dir.display(), old)) };
        let result = f();
        unsafe { std::env::set_var("PATH", old) };
        result
    }

    fn with_path_only<T>(dir: &Path, f: impl FnOnce() -> T) -> T {
        let old = std::env::var("PATH").ok();
        unsafe { std::env::set_var("PATH", dir) };
        let result = f();
        match old {
            Some(v) => unsafe { std::env::set_var("PATH", v) },
            None => unsafe { std::env::remove_var("PATH") },
        }
        result
    }

    fn drive(fut: impl Future<Output = Result<(), String>>) -> Result<(), String> {
        glib::MainContext::default().block_on(fut)
    }

    /// A real PolicyKit dialog can't run in tests; a PATH-shimmed `pkexec`
    /// exercises stdin streaming, exit-status mapping, and the
    /// not-installed error instead.
    #[test]
    fn elevated_write_success_failure_and_missing_binary() {
        let dir = std::env::temp_dir().join(format!("gedit-rs-elev-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("out.txt");

        // Success: the shim plays pkexec (runs its first argument with the
        // rest as args), so `shift` hands the remaining `-- <path>` to real
        // `tee`, which consumes stdin.
        write_fake_pkexec(&dir, "#!/bin/sh\nshift\nexec tee \"$@\"\n");
        let result = with_path_prepend(&dir, || drive(elevated_write(&out, "root content\n")));
        assert_eq!(result, Ok(()));
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "root content\n");

        // Dismissed dialog: non-zero exit with stderr detail; target intact.
        write_fake_pkexec(&dir, "#!/bin/sh\necho 'Authentication dismissed' >&2\nexit 1\n");
        let err = with_path_prepend(&dir, || drive(elevated_write(&out, "nope"))).unwrap_err();
        assert!(err.contains("Authentication dismissed"), "{err}");
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "root content\n");

        // No pkexec on PATH at all.
        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let err = with_path_only(&empty, || drive(elevated_write(&out, "nope"))).unwrap_err();
        assert!(err.contains("pkexec is not installed"), "{err}");

        std::fs::remove_dir_all(&dir).ok();
    }
}
