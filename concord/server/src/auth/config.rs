/// Authentication settings produced by the validated top-level configuration loader.
///
/// This type deliberately has no environment or file loader of its own. Keeping loading in
/// `crate::config` prevents authentication from observing a different configuration snapshot.
#[derive(Clone)]
pub struct AuthConfig {
    pub jwt_secret: String,
    pub session_expiry_hours: i64,
    pub public_url: String,
}
