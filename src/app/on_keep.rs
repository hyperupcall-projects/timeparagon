//! The Google Keep screen.

use super::*;

pub(super) fn update(state: &mut State, message: KeepMessage) -> Task<Message> {
    match message {
        KeepMessage::KeepRefresh => refresh_keep(state),
        KeepMessage::KeepSignin => {
            if state.keep.loading {
                return Task::none();
            }
            state.keep.loading = true;
            state.keep.error = None;
            blocking::run(crate::keep::signin)
                .map(|v0| Message::Keep(KeepMessage::KeepSignedIn(v0)))
        }
        KeepMessage::KeepSignedIn(result) => {
            state.keep.loading = false;
            match result {
                Ok(()) => refresh_keep(state),
                Err(error) => {
                    state.keep.error = Some(error);
                    Task::none()
                }
            }
        }
        KeepMessage::KeepLoaded(result) => {
            state.keep.loading = false;
            match result {
                Ok(snapshot) => {
                    state.keep.snapshot = snapshot;
                    state.keep.error = None;
                }
                Err(error) => state.keep.error = Some(error),
            }
            Task::none()
        }
        KeepMessage::KeepQueryChanged(query) => {
            state.keep.query = query;
            Task::none()
        }
        KeepMessage::KeepLabelChanged(label) => {
            state.keep.label = label;
            Task::none()
        }
        KeepMessage::KeepScopeChanged(scope) => {
            state.keep.scope = scope;
            Task::none()
        }
        KeepMessage::KeepEdit(url) => {
            if url.starts_with("https://keep.google.com/")
                && let Err(error) = calendar::oauth::open_in_browser(&url)
            {
                state.keep.error = Some(error);
            }
            Task::none()
        }
    }
}
