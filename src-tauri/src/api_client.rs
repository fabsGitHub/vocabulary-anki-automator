use serde::Deserialize;

#[derive(Deserialize)]
struct DatamuseWord {
    tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct DatamuseSynonym {
    word: String,
}

pub struct ApiClient {
    client: reqwest::Client,
}

impl ApiClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    /// ✨ NEW: 100% Free Keyless Translation via Google's Unofficial Client Endpoint
    pub async fn translate(&self, text: &str, source_lang: &str, target_lang: &str) -> String {
        let sl = source_lang.to_lowercase();
        let tl = target_lang.to_lowercase();

        // Construct the keyless query string array payload
        let params = [
            ("client", "gtx"),
            ("sl", &sl),
            ("tl", &tl),
            ("dt", "t"),
            ("q", text),
        ];

        let url = match reqwest::Url::parse_with_params(
            "https://translate.googleapis.com/translate_a/single",
            &params,
        ) {
            Ok(u) => u,
            Err(_) => return String::new(),
        };

        if let Ok(res) = self.client.get(url).send().await {
            if let Ok(json) = res.json::<serde_json::Value>().await {
                // Google returns nested matrix arrays: [[["Translated Text", "Original Text", ...]]]
                if let Some(outer_arr) = json.get(0).and_then(|v| v.as_array()) {
                    let mut full_translation = String::new();
                    for item in outer_arr {
                        if let Some(trans_val) = item.get(0).and_then(|v| v.as_str()) {
                            full_translation.push_str(trans_val);
                        }
                    }
                    if !full_translation.is_empty() {
                        return full_translation;
                    }
                }
            }
        }
        String::new()
    }

    /// Persistent Datamuse Word Metric Analysis Loop (Always Free)
    pub async fn analyze_rarity(&self, word: &str) -> (bool, Option<String>) {
        let clean_word = word.trim().to_lowercase();
        if clean_word.is_empty() || clean_word.contains(' ') {
            return (false, None);
        }

        let url = format!(
            "https://api.datamuse.com/words?sp={}&md=f&max=1",
            clean_word
        );
        if let Ok(res) = self.client.get(&url).send().await {
            if let Ok(words) = res.json::<Vec<DatamuseWord>>().await {
                if let Some(first_word) = words.first() {
                    let mut freq = 0.0;
                    if let Some(tags) = &first_word.tags {
                        for tag in tags {
                            if tag.starts_with("f:") {
                                if let Ok(parsed_freq) = tag["f:".len()..].parse::<f64>() {
                                    freq = parsed_freq;
                                }
                            }
                        }
                    }

                    if freq < 5.0 {
                        let syn_url = format!(
                            "https://api.datamuse.com/words?rel_syn={}&max=1",
                            clean_word
                        );
                        if let Ok(syn_res) = self.client.get(&syn_url).send().await {
                            if let Ok(syns) = syn_res.json::<Vec<DatamuseSynonym>>().await {
                                if let Some(syn) = syns.first() {
                                    return (true, Some(syn.word.clone()));
                                }
                            }
                        }
                        return (true, None);
                    }
                }
            }
        }
        (false, None)
    }

    /// Public Dictionary Example Context Sentence Parsing Engine (Always Free)
    pub async fn fetch_example(&self, word: &str) -> String {
        let clean_word = word.trim().to_lowercase();
        let url = format!(
            "https://api.dictionaryapi.dev/api/v2/entries/en/{}",
            clean_word
        );

        if let Ok(res) = self.client.get(&url).send().await {
            if let Ok(json) = res.json::<serde_json::Value>().await {
                if let Some(meanings) = json
                    .get(0)
                    .and_then(|v| v.get("meanings"))
                    .and_then(|v| v.as_array())
                {
                    for meaning in meanings {
                        if let Some(definitions) =
                            meaning.get("definitions").and_then(|v| v.as_array())
                        {
                            for definition in definitions {
                                if let Some(example) =
                                    definition.get("example").and_then(|v| v.as_str())
                                {
                                    return example.to_string();
                                }
                            }
                        }
                    }
                }
            }
        }
        format!("We use the expression '{}' in everyday context.", word)
    }
}
