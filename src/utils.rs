//! Tiny helpers: awaitable dialog response and blocking error popups.

use gtk::prelude::*;
use gtk::ResponseType;
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
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
