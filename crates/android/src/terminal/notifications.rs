//! Transient notification delivery, independent of conflated screen frames.

use std::sync::Mutex;

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use zterm_core::terminal::{TerminalNotification, TerminalNotificationQueue};

use crate::NativeError;

/// One consumed notification with an opaque attachment-lifetime fence.
#[derive(Clone, uniffi::Record)]
pub struct NativeNotification {
    /// Check against NativeTerminal immediately before invoking the platform API.
    pub generation: u64,
    /// None for OSC 9; an optionally empty title for OSC 777.
    pub title: Option<String>,
    /// Complete OSC 9 message or OSC 777 body.
    pub body: String,
}

pub(super) struct NotificationBridge {
    state: Mutex<State>,
    wake: watch::Sender<()>,
}

struct State {
    generation: u64,
    connected: bool,
    pending: TerminalNotificationQueue,
}

impl NotificationBridge {
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(State {
                generation: 1,
                connected: false,
                pending: Default::default(),
            }),
            wake: watch::channel(()).0,
        }
    }

    pub(super) fn connected(&self, connected: bool) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !connected {
            state.pending.clear();
            if state.connected && state.generation != 0 {
                state.generation = state.generation.checked_add(1).unwrap_or(0);
            }
        }
        state.connected = connected && state.generation != 0;
    }

    pub(super) fn is_current(&self, generation: u64) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.connected && state.generation == generation
    }

    pub(super) fn push(&self, value: TerminalNotification) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.connected {
            state.pending.push(value);
            self.wake.send_replace(());
        }
    }

    pub(super) async fn next(
        &self,
        cancel: &CancellationToken,
    ) -> Result<NativeNotification, NativeError> {
        let mut wake = self.wake.subscribe();
        loop {
            if cancel.is_cancelled() {
                return Err(NativeError::Closed);
            }
            wake.borrow_and_update();
            {
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(value) = state.pending.pop() {
                    return Ok(NativeNotification {
                        generation: state.generation,
                        title: value.title().map(str::to_owned),
                        body: value.body().to_owned(),
                    });
                }
            }
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(NativeError::Closed),
                changed = wake.changed() => { changed.map_err(|_| NativeError::Closed)?; }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification(index: usize) -> TerminalNotification {
        TerminalNotification::osc9(format!("notification {index}")).expect("valid notification")
    }

    #[tokio::test]
    async fn bounded_events_are_distinct_and_retired_across_same_handle_reconnect() {
        let bridge = NotificationBridge::new();
        let cancel = CancellationToken::new();
        bridge.push(notification(0)); // No initial controller yet.
        bridge.connected(true);
        for index in 1..=40 {
            bridge.push(notification(index));
        }
        let first = bridge.next(&cancel).await.expect("first event");
        assert_eq!(first.body, "notification 9");
        assert!(bridge.is_current(first.generation));
        // Reading the next event needs no second watch publication.
        assert_eq!(
            bridge.next(&cancel).await.expect("second event").body,
            "notification 10"
        );
        bridge.connected(false);
        bridge.push(notification(41)); // Disconnected output is never retained.
        bridge.connected(true);
        assert!(!bridge.is_current(first.generation));
        bridge.push(notification(42));
        let fresh = bridge.next(&cancel).await.expect("fresh event");
        assert_eq!(fresh.body, "notification 42");
        assert!(bridge.is_current(fresh.generation));
        assert_ne!(fresh.generation, first.generation);
        bridge.connected(false);
        assert!(!bridge.is_current(fresh.generation));
        cancel.cancel();
        assert!(matches!(
            bridge.next(&cancel).await,
            Err(NativeError::Closed)
        ));
    }

    #[tokio::test]
    async fn cancelling_waiter_preserves_future_delivery_and_close_wakes_it() {
        let bridge = NotificationBridge::new();
        bridge.connected(true);
        let cancel = CancellationToken::new();
        let mut wait = Box::pin(bridge.next(&cancel));
        tokio::select! {
            biased;
            _ = &mut wait => panic!("no pending notification"),
            () = std::future::ready(()) => {}
        }
        drop(wait);
        bridge.push(notification(1));
        assert_eq!(
            bridge.next(&cancel).await.expect("future event").body,
            "notification 1"
        );
        let wait = bridge.next(&cancel);
        let close = async {
            tokio::task::yield_now().await;
            cancel.cancel();
        };
        let (result, ()) = tokio::join!(wait, close);
        assert!(matches!(result, Err(NativeError::Closed)));
    }
}
