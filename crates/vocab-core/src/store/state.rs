//! `app_state`: small string settings that belong to the shared database
//! rather than to one app's `config.json`, because both apps read them
//! (`curriculum_fingerprint`, `last_prompted_date`,
//! `streak_freezes_available`).

use rusqlite::OptionalExtension;

use super::{Store, StoreResult};

impl Store {
    pub fn get_state(&self, key: &str) -> StoreResult<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT value FROM app_state WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    /// Sets `key` to `value`, replacing any previous value.
    pub fn set_state(&self, key: &str, value: &str) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO app_state (key, value) VALUES (?1, ?2) \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }

    /// Removes `key`; removing a key that is not set is a no-op.
    pub fn delete_state(&self, key: &str) -> StoreResult<()> {
        self.conn()
            .execute("DELETE FROM app_state WHERE key = ?1", [key])?;
        Ok(())
    }
}
