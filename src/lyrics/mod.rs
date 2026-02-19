use serde_json::Value;

pub fn fetch_lyrics(title: &str, artist: &str) -> Option<String> {
    let query = if artist.trim().is_empty() {
        title.trim().to_string()
    } else {
        format!("{} {}", title.trim(), artist.trim())
    };

    if query.is_empty() {
        return None;
    }

    let response = ureq::get("https://lrclib.net/api/search")
        .query("q", &query)
        .call()
        .ok()?;
    let payload: Value = response.into_json().ok()?;
    let entries = payload.as_array()?;
    let first = entries.first()?;
    first
        .get("plainLyrics")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}
