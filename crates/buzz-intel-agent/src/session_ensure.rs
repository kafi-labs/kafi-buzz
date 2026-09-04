//! Single-flight get-or-create for intel session mappings.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::Mutex;

use crate::error::AdapterError;
use crate::intel::CreateSessionResponse;
use crate::state::{SessionEntry, StateStore};

/// Per-key locks so concurrent ensure calls for one mapping_key create once.
pub type CreateLockMap = Mutex<HashMap<String, Arc<Mutex<()>>>>;

/// Look up or create an intel session for `mapping_key`, single-flight.
///
/// Returns `(session_id, is_new, system_prompt_already_forwarded)`.
///
/// When `force_new` is true the existing mapping is dropped before create.
pub async fn get_or_create_intel_session<C, Fut>(
    state: &Mutex<StateStore>,
    create_locks: &CreateLockMap,
    mapping_key: &str,
    force_new: bool,
    entity_id: &str,
    mut create: C,
) -> Result<(String, bool, bool), AdapterError>
where
    C: FnMut() -> Fut,
    Fut: Future<Output = Result<CreateSessionResponse, AdapterError>>,
{
    // Fast path: hit without taking a create lock.
    if !force_new {
        let state_guard = state.lock().await;
        if let Some(entry) = state_guard.get_session(mapping_key) {
            return Ok((
                entry.session_id.clone(),
                false,
                entry.system_prompt_forwarded,
            ));
        }
    }

    // Single-flight: only one creator per key at a time.
    let key_lock = {
        let mut locks = create_locks.lock().await;
        locks
            .entry(mapping_key.to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _flight = key_lock.lock().await;

    // Re-check under the create lock (loser reuses the winner's session).
    {
        let mut state_guard = state.lock().await;
        if force_new {
            let _ = state_guard.remove_session(mapping_key);
        } else if let Some(entry) = state_guard.get_session(mapping_key) {
            return Ok((
                entry.session_id.clone(),
                false,
                entry.system_prompt_forwarded,
            ));
        }
    }

    let created = create().await?;
    let entry = SessionEntry {
        session_id: created.session_id.clone(),
        entity_id: entity_id.to_owned(),
        created_at: Utc::now(),
        last_used_at: Utc::now(),
        system_prompt_forwarded: false,
    };
    {
        let mut state_guard = state.lock().await;
        // If another path inserted meanwhile, prefer the map (shouldn't happen
        // under the create lock, but be defensive).
        if !force_new {
            if let Some(existing) = state_guard.get_session(mapping_key) {
                return Ok((
                    existing.session_id.clone(),
                    false,
                    existing.system_prompt_forwarded,
                ));
            }
        }
        state_guard.put_session(mapping_key.to_owned(), entry)?;
    }
    Ok((created.session_id, true, false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    #[tokio::test]
    async fn concurrent_ensure_creates_once() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");
        let state = Arc::new(Mutex::new(StateStore::load(&path).unwrap()));
        let locks = Arc::new(Mutex::new(HashMap::new()));
        let creates = Arc::new(AtomicUsize::new(0));

        let key = "channel-1";
        let entity = "buzz:channel:channel-1";

        let mut handles = Vec::new();
        for i in 0..8 {
            let state = state.clone();
            let locks = locks.clone();
            let creates = creates.clone();
            handles.push(tokio::spawn(async move {
                // Stagger slightly so multiple miss the fast path.
                if i > 0 {
                    tokio::task::yield_now().await;
                }
                get_or_create_intel_session(
                    state.as_ref(),
                    locks.as_ref(),
                    key,
                    false,
                    entity,
                    || {
                        let creates = creates.clone();
                        async move {
                            creates.fetch_add(1, Ordering::SeqCst);
                            // Simulate slow gateway.
                            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                            Ok(CreateSessionResponse {
                                session_id: "sess-unique".into(),
                                created_at: None,
                            })
                        }
                    },
                )
                .await
            }));
        }

        let mut session_ids = Vec::new();
        for h in handles {
            let (sid, _, _) = h.await.unwrap().unwrap();
            session_ids.push(sid);
        }
        assert_eq!(
            creates.load(Ordering::SeqCst),
            1,
            "expected exactly one create"
        );
        assert!(session_ids.iter().all(|s| s == "sess-unique"));
    }
}
