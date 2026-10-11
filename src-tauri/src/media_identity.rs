use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaIdentity {
    #[serde(alias = "webpage_url")]
    pub source_url: Option<String>,
    pub title: Option<String>,
    pub thumbnail: Option<String>,
    pub uploader: Option<String>,
    pub extractor: Option<String>,
}

impl MediaIdentity {
    pub fn from_metadata(value: &serde_json::Value) -> Self {
        let mut identity: Self = serde_json::from_value(value.clone()).unwrap_or_default();
        // Never persist arbitrary schemes returned by an extractor.
        identity.source_url = identity.source_url.filter(|value| {
            url::Url::parse(value).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
        });
        identity
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn playlist_outputs_keep_individual_identity() {
        let first = MediaIdentity::from_metadata(
            &serde_json::json!({"webpage_url":"https://example.com/video/1", "title":"Перше"}),
        );
        let second = MediaIdentity::from_metadata(
            &serde_json::json!({"webpage_url":"https://example.com/video/2", "title":"Друге"}),
        );
        assert_ne!(first.source_url, second.source_url);
        assert_eq!(second.title.as_deref(), Some("Друге"));
        assert_eq!(
            serde_json::to_value(second).unwrap()["sourceUrl"],
            "https://example.com/video/2"
        );
    }
    #[test]
    fn rejects_executable_source_scheme() {
        assert!(MediaIdentity::from_metadata(
            &serde_json::json!({"webpage_url":"javascript:alert(1)"})
        )
        .source_url
        .is_none());
    }
}
