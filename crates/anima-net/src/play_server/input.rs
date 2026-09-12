//! Bind queued input to the connection the caller actually saw. Checking only
//! when accepting HTTP would still let already-queued commands cross a reconnect.
use super::*;

pub(super) struct SessionInput {
    session_id: String,
    action: Option<Action>,
}
impl SessionInput {
    pub(super) fn for_session(self, id: &str) -> Option<Option<Action>> {
        (self.session_id == id).then_some(self.action)
    }
}

#[derive(Clone)]
pub(super) struct InputSender {
    active: Arc<Mutex<Option<String>>>,
    tx: mpsc::Sender<SessionInput>,
}
impl InputSender {
    pub(super) fn channel() -> (Self, mpsc::Receiver<SessionInput>) {
        let (tx, rx) = mpsc::channel();
        (
            Self {
                active: Arc::new(Mutex::new(None)),
                tx,
            },
            rx,
        )
    }
    pub(super) fn activate(&self, id: &str) -> ActiveInput {
        *self.active.lock().unwrap() = Some(id.to_owned());
        ActiveInput {
            active: self.active.clone(),
            id: id.to_owned(),
        }
    }
    pub(super) fn send(
        &self,
        id: Option<&str>,
        action: Option<Action>,
    ) -> Result<(), &'static str> {
        let active = self.active.lock().unwrap();
        let id = id
            .filter(|id| !id.is_empty())
            .ok_or("Refresh the scene before sending input.")?;
        if active.as_deref() != Some(id) {
            return Err("This game session has ended. Refresh the scene.");
        }
        self.tx
            .send(SessionInput {
                session_id: id.to_owned(),
                action,
            })
            .map_err(|_| "The game session is unavailable.")
    }
}

/// Clears acceptance on every session exit, including early returns and unwind.
pub(super) struct ActiveInput {
    active: Arc<Mutex<Option<String>>>,
    id: String,
}
impl Drop for ActiveInput {
    fn drop(&mut self) {
        let mut active = self.active.lock().unwrap();
        if active.as_deref() == Some(&self.id) {
            *active = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unbound_and_ended_sessions_including_stop() {
        let (tx, rx) = InputSender::channel();
        assert!(tx.send(None, None).is_err());
        assert!(tx.send(Some("old"), None).is_err());
        let active = tx.activate("current");
        assert!(tx
            .send(None, Some(Action::Walk { dir: 1, run: false }))
            .is_err());
        assert!(tx.send(Some("old"), None).is_err());
        assert!(rx.try_recv().is_err());
        tx.send(Some("current"), None).unwrap();
        assert!(matches!(
            rx.try_recv().unwrap().for_session("current"),
            Some(None)
        ));
        drop(active);
        assert!(tx.send(Some("current"), None).is_err());
    }

    #[test]
    fn accepted_but_queued_input_never_crosses_a_reconnect() {
        let (tx, rx) = InputSender::channel();
        let old = tx.activate("old");
        tx.send(Some("old"), Some(Action::Walk { dir: 2, run: true }))
            .unwrap();
        tx.send(Some("old"), None).unwrap();
        let current = tx.activate("new");
        drop(old); // an old guard cannot close its replacement
        tx.send(Some("new"), Some(Action::Walk { dir: 5, run: false }))
            .unwrap();
        assert!(rx.try_recv().unwrap().for_session("new").is_none());
        assert!(rx.try_recv().unwrap().for_session("new").is_none());
        assert!(matches!(
            rx.try_recv().unwrap().for_session("new"),
            Some(Some(Action::Walk { dir: 5, run: false }))
        ));
        drop(current);
        assert!(tx.send(Some("new"), None).is_err());
    }
}
