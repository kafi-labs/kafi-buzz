//! Post kind-9 replies to the Buzz relay via buzz-sdk + HTTP POST /events.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use buzz_sdk::ThreadRef;
use nostr::{EventBuilder, EventId, JsonUtil, Keys, Kind, Tag};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::chunk::chunk_content;
use crate::config::normalize_relay_url;
use crate::error::AdapterError;

/// Buzz relay publisher (signing + POST /events + optional query for thread root).
pub struct RelayPublisher {
    http: reqwest::Client,
    relay_url: String,
    keys: Keys,
    auth_tag: Option<Tag>,
    auth_tag_json: Option<String>,
}

impl RelayPublisher {
    /// Construct from relay URL, private key string, and optional auth tag JSON.
    pub fn new(
        relay_url: &str,
        private_key: &str,
        auth_tag_json: Option<&str>,
    ) -> Result<Self, AdapterError> {
        let keys = Keys::parse(private_key)
            .map_err(|e| AdapterError::Config(format!("invalid BUZZ_PRIVATE_KEY: {e}")))?;

        let (auth_tag, auth_tag_json) = match auth_tag_json {
            Some(json) if !json.is_empty() => {
                let tag = buzz_sdk::nip_oa::parse_auth_tag(json).map_err(|e| {
                    AdapterError::Config(format!("BUZZ_AUTH_TAG is malformed: {e}"))
                })?;
                buzz_sdk::nip_oa::verify_auth_tag(json, &keys.public_key()).map_err(|e| {
                    AdapterError::Config(format!(
                        "BUZZ_AUTH_TAG verification failed for pubkey {}: {e}",
                        keys.public_key().to_hex()
                    ))
                })?;
                (Some(tag), Some(json.to_owned()))
            }
            _ => (None, None),
        };

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| AdapterError::Relay(e.to_string()))?;

        Ok(Self {
            http,
            relay_url: normalize_relay_url(relay_url),
            keys,
            auth_tag,
            auth_tag_json,
        })
    }

    /// Owner pubkey hex from the NIP-OA auth tag, if present.
    pub fn owner_pubkey_hex(&self) -> Option<String> {
        self.auth_tag
            .as_ref()
            .map(|t| t.as_slice())
            .and_then(|s| s.get(1).cloned())
    }

    /// Agent (this key) pubkey hex.
    pub fn agent_pubkey_hex(&self) -> String {
        self.keys.public_key().to_hex()
    }

    /// Publish a kind-9 message, optionally threaded. Chunks oversized content.
    ///
    /// Returns the first published event id (hex), if anything was posted.
    pub async fn post_message(
        &self,
        channel_id: Uuid,
        content: &str,
        reply_to: Option<&str>,
    ) -> Result<Option<String>, AdapterError> {
        let chunks = chunk_content(content);
        if chunks.is_empty() {
            return Ok(None);
        }

        let thread_ref = match reply_to {
            Some(parent) => Some(self.resolve_thread_ref(parent).await?),
            None => None,
        };

        let mut first_event_id: Option<String> = None;
        let mut first_eid: Option<EventId> = None;
        let mut previous_chunk_id: Option<EventId> = None;

        for (i, chunk) in chunks.iter().enumerate() {
            let followup: Option<ThreadRef> = if i == 0 {
                None
            } else if let (Some(root), Some(parent)) = (first_eid, previous_chunk_id) {
                Some(ThreadRef {
                    root_event_id: root,
                    parent_event_id: parent,
                })
            } else {
                None
            };
            let effective: Option<&ThreadRef> = if i == 0 {
                thread_ref.as_ref()
            } else {
                followup.as_ref()
            };

            let builder =
                buzz_sdk::build_message(channel_id, chunk, effective, &[], false, &[], &[])
                    .map_err(|e| AdapterError::Relay(format!("build_message: {e}")))?;

            let event = self.sign_event(builder)?;
            let event_id = event.id.to_hex();
            let eid = event.id;
            self.submit_event(event).await?;

            if first_event_id.is_none() {
                first_event_id = Some(event_id);
                first_eid = Some(eid);
            }
            previous_chunk_id = Some(eid);
        }

        Ok(first_event_id)
    }

    fn sign_event(&self, builder: EventBuilder) -> Result<nostr::Event, AdapterError> {
        let builder = if let Some(ref tag) = self.auth_tag {
            builder.tags([tag.clone()])
        } else {
            builder
        };
        builder
            .sign_with_keys(&self.keys)
            .map_err(|e| AdapterError::Relay(format!("signing failed: {e}")))
    }

    async fn submit_event(&self, event: nostr::Event) -> Result<(), AdapterError> {
        let url = format!("{}/events", self.relay_url);
        let body = serde_json::to_vec(&event)
            .map_err(|e| AdapterError::Relay(format!("serialize event: {e}")))?;
        let auth = sign_nip98(&self.keys, "POST", &url, Some(&body))?;
        let mut req = self
            .http
            .post(&url)
            .header("Authorization", auth)
            .header("Content-Type", "application/json")
            .body(body);
        if let Some(ref json) = self.auth_tag_json {
            req = req.header("x-auth-tag", json);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| AdapterError::Relay(format!("POST /events: {e}")))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(AdapterError::Relay(format!(
                "POST /events status {}: {}",
                status.as_u16(),
                text.chars().take(200).collect::<String>()
            )));
        }
        Ok(())
    }

    /// Resolve NIP-10 thread root from the parent event (falls back to parent-as-root).
    async fn resolve_thread_ref(&self, parent_event_id: &str) -> Result<ThreadRef, AdapterError> {
        let parent_eid = EventId::from_hex(parent_event_id)
            .map_err(|e| AdapterError::Relay(format!("invalid reply-to event id: {e}")))?;

        match self.query_event_tags(parent_event_id).await {
            Ok(tags) => {
                let root = find_root_from_tags(&tags)
                    .and_then(|h| EventId::from_hex(&h).ok())
                    .filter(|r| *r != parent_eid)
                    .unwrap_or(parent_eid);
                Ok(ThreadRef {
                    root_event_id: root,
                    parent_event_id: parent_eid,
                })
            }
            Err(e) => {
                tracing::warn!(
                    "reply: could not resolve thread root for {parent_event_id}: {e}; using direct reply"
                );
                Ok(ThreadRef {
                    root_event_id: parent_eid,
                    parent_event_id: parent_eid,
                })
            }
        }
    }

    async fn query_event_tags(&self, event_id: &str) -> Result<serde_json::Value, AdapterError> {
        let url = format!("{}/query", self.relay_url);
        let filter = serde_json::json!([{ "ids": [event_id], "limit": 1 }]);
        let body = serde_json::to_vec(&filter)
            .map_err(|e| AdapterError::Relay(format!("serialize query: {e}")))?;
        let auth = sign_nip98(&self.keys, "POST", &url, Some(&body))?;
        let mut req = self
            .http
            .post(&url)
            .header("Authorization", auth)
            .header("Content-Type", "application/json")
            .body(body);
        if let Some(ref json) = self.auth_tag_json {
            req = req.header("x-auth-tag", json);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| AdapterError::Relay(format!("POST /query: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| AdapterError::Relay(format!("query body: {e}")))?;
        if !status.is_success() {
            return Err(AdapterError::Relay(format!(
                "POST /query status {}: {}",
                status.as_u16(),
                text.chars().take(200).collect::<String>()
            )));
        }
        let events: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| AdapterError::Relay(format!("query json: {e}")))?;
        let event = events
            .as_array()
            .and_then(|a| a.first())
            .ok_or_else(|| AdapterError::Relay(format!("parent event {event_id} not found")))?;
        Ok(event
            .get("tags")
            .cloned()
            .unwrap_or(serde_json::Value::Null))
    }
}

fn find_root_from_tags(tags: &serde_json::Value) -> Option<String> {
    let arr = tags.as_array()?;
    // Prefer e-tag with marker "root".
    for tag in arr {
        let t = tag.as_array()?;
        if t.first().and_then(|v| v.as_str()) != Some("e") {
            continue;
        }
        let id = t.get(1).and_then(|v| v.as_str())?;
        let marker = t.get(3).and_then(|v| v.as_str()).unwrap_or("");
        if marker == "root" {
            return Some(id.to_owned());
        }
    }
    // Some events only have reply marker; root is the parent itself (handled by caller).
    None
}

fn sign_nip98(
    keys: &Keys,
    method: &str,
    url: &str,
    body: Option<&[u8]>,
) -> Result<String, AdapterError> {
    let mut tags = vec![
        Tag::parse(["u", url]).map_err(|e| AdapterError::Relay(format!("tag error: {e}")))?,
        Tag::parse(["method", method])
            .map_err(|e| AdapterError::Relay(format!("tag error: {e}")))?,
        Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()])
            .map_err(|e| AdapterError::Relay(format!("tag error: {e}")))?,
    ];
    if let Some(b) = body {
        let hash = hex::encode(Sha256::digest(b));
        tags.push(
            Tag::parse(["payload", &hash])
                .map_err(|e| AdapterError::Relay(format!("tag error: {e}")))?,
        );
    }
    let event = EventBuilder::new(Kind::Custom(27235), "")
        .tags(tags)
        .sign_with_keys(keys)
        .map_err(|e| AdapterError::Relay(format!("NIP-98 sign: {e}")))?;
    let json = event.as_json();
    Ok(format!("Nostr {}", B64.encode(json.as_bytes())))
}
