use super::{Deserialize, Serialize, provider_get_json};

pub(super) async fn fetch_bsky_profile(
    transport: &crate::egress::ControlledHttpClient,
    did: &str,
) -> (Option<String>, Option<String>) {
    #[derive(Deserialize)]
    struct Profile {
        #[serde(rename = "displayName")]
        display_name: Option<String>,
        avatar: Option<String>,
        handle: Option<String>,
    }
    let Ok(url) = reqwest::Url::parse_with_params(
        "https://public.api.bsky.app/xrpc/app.bsky.actor.getProfile",
        &[("actor", did)],
    ) else {
        return (None, None);
    };
    match provider_get_json::<Profile>(transport, url).await {
        Ok(profile) => (
            profile
                .display_name
                .filter(|name| !name.is_empty())
                .or(profile.handle),
            profile.avatar,
        ),
        Err(_) => (None, None),
    }
}

/// Full Bluesky profile data returned by `fetch_full_bsky_profile()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueskyProfile {
    pub did: String,
    pub handle: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub avatar: Option<String>,
    pub banner: Option<String>,
    pub followers_count: i64,
    pub follows_count: i64,
    pub posts_count: i64,
}

/// Fetch a full public Bluesky profile via `app.bsky.actor.getProfile`.
/// Returns `None` if the profile cannot be fetched.
pub async fn fetch_full_bsky_profile(
    transport: &crate::egress::ControlledHttpClient,
    endpoint: &reqwest::Url,
    did: &str,
) -> Option<BlueskyProfile> {
    #[derive(Deserialize)]
    struct RawProfile {
        did: String,
        handle: String,
        #[serde(rename = "displayName")]
        display_name: Option<String>,
        description: Option<String>,
        avatar: Option<String>,
        banner: Option<String>,
        #[serde(rename = "followersCount", default)]
        followers_count: i64,
        #[serde(rename = "followsCount", default)]
        follows_count: i64,
        #[serde(rename = "postsCount", default)]
        posts_count: i64,
    }

    let mut url = endpoint.clone();
    url.query_pairs_mut().append_pair("actor", did);
    let raw: RawProfile = provider_get_json(transport, url).await.ok()?;
    Some(BlueskyProfile {
        did: raw.did,
        handle: raw.handle,
        display_name: raw.display_name,
        description: raw.description,
        avatar: raw.avatar,
        banner: raw.banner,
        followers_count: raw.followers_count,
        follows_count: raw.follows_count,
        posts_count: raw.posts_count,
    })
}
