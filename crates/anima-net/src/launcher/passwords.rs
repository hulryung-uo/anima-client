//! Stage vault changes until profile bytes are ready, undo on a failed commit.
use super::PasswordVault;

#[derive(Default)]
pub(super) struct PasswordChanges(Vec<(String, Option<String>)>);

impl PasswordChanges {
    pub fn set(&mut self, key: String, value: Option<String>) {
        if let Some((_, pending)) = self.0.iter_mut().find(|(k, _)| *k == key) {
            *pending = value;
        } else {
            self.0.push((key, value));
        }
    }

    pub fn apply<'a>(
        &self,
        vault: Option<&'a dyn PasswordVault>,
    ) -> Result<AppliedPasswords<'a>, String> {
        let mut applied = AppliedPasswords {
            vault,
            previous: vec![],
        };
        if self.0.is_empty() {
            return Ok(applied);
        }
        let vault = vault.ok_or("Open the desktop app to update saved passwords.")?;
        // Read every original before the first mutation. A locked store must
        // not leave a partially deleted set of accounts.
        let originals = self
            .0
            .iter()
            .map(|(key, _)| vault.get(key))
            .collect::<Result<Vec<_>, _>>()?;
        for ((key, value), previous) in self.0.iter().zip(originals) {
            // Include even the failed operation: an OS call may change the
            // entry before reporting an error.
            applied.previous.push((key.clone(), previous));
            if let Err(error) = write(vault, key, value.as_deref()) {
                return Err(applied.fail(error));
            }
        }
        Ok(applied)
    }
}

pub(super) struct AppliedPasswords<'a> {
    vault: Option<&'a dyn PasswordVault>,
    previous: Vec<(String, Option<String>)>,
}
impl AppliedPasswords<'_> {
    pub fn commit(mut self) {
        self.previous.clear();
    }

    pub fn fail(mut self, error: String) -> String {
        if self.restore() {
            error
        } else {
            format!("{error} Some saved passwords could not be restored. Unlock the system password store, then enter and save any missing or changed passwords again.")
        }
    }

    fn restore(&mut self) -> bool {
        let Some(vault) = self.vault else { return true };
        let mut restored = true;
        for (key, value) in self.previous.drain(..).rev() {
            // A failing operation may have left the original untouched; avoid
            // an unnecessary write (especially when the vault is locked).
            if matches!(vault.get(&key), Ok(current) if current == value) {
                continue;
            }
            if write(vault, &key, value.as_deref()).is_err() {
                restored = false;
            }
        }
        restored
    }
}
impl Drop for AppliedPasswords<'_> {
    fn drop(&mut self) {
        self.restore();
    }
}

fn write(vault: &dyn PasswordVault, key: &str, value: Option<&str>) -> Result<(), String> {
    match value {
        Some(password) => vault.set(key, password),
        None => vault.delete(key),
    }
}
