use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::json;

pub struct AnkiClient {
    client: reqwest::Client,
    url: String,
}

impl AnkiClient {
    pub fn new(url: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            url,
        }
    }

    async fn invoke(
        &self,
        action: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let payload = json!({
            "action": action,
            "version": 6,
            "params": params
        });

        let resp = self
            .client
            .post(&self.url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        if let Some(error) = body.get("error").and_then(|e| e.as_str()) {
            return Err(error.to_string());
        }
        Ok(body
            .get("result")
            .cloned()
            .unwrap_or(serde_json::Value::Null))
    }

    // ✨ FIXED: Updated signature to accept 8 parameters
    pub async fn add_vocab_card(
        &self,
        deck: &str,
        model: &str,
        front: &str,
        back: &str,
        example: &str,
        note: &str,
        source_lang: &str,
        _target_lang: &str,
    ) -> Result<(), String> {
        let _ = self.invoke("createDeck", json!({ "deck": deck })).await;

        // Dynamic TTS voice generation using the actual source language attribute
        let tts_url = format!(
            "https://translate.google.com/translate_tts?ie=UTF-8&tl={}&client=tw-ob&q={}",
            source_lang.to_lowercase(),
            urlencoding::encode(front)
        );

        let mut audio_file_payload = json!({});
        if let Ok(tts_resp) = self
            .client
            .get(&tts_url)
            .header("User-Agent", "Mozilla/5.0")
            .send()
            .await
        {
            if let Ok(bytes) = tts_resp.bytes().await {
                let b64_encoded = STANDARD.encode(bytes);
                let filename = format!("automator_{}.mp3", uuid::Uuid::new_v4());
                audio_file_payload = json!({
                    "url": tts_url,
                    "filename": filename,
                    "fields": ["Front Audio"]
                });

                let _ = self
                    .invoke(
                        "storeMediaFile",
                        json!({
                            "filename": filename,
                            "data": b64_encoded
                        }),
                    )
                    .await;
            }
        }

        let fields = json!({
            "Front": front,
            "Back": back,
            "Context Sentence": example,
            "Notes": note
        });

        let note_payload = json!({
            "note": {
                "deckName": deck,
                "modelName": model,
                "fields": fields,
                "options": {
                    "allowDuplicate": false,
                    "duplicateScope": "deck"
                },
                "audio": [audio_file_payload]
            }
        });

        self.invoke("addNote", note_payload).await?;
        Ok(())
    }
}
