# Delegated OAuth lifecycle

Consent, client validation, token issue, refresh rotation, access checks, and logout have separate owners. ../oauth.rs exports the stable route functions.

Recheck browser credential, application, grant scope, and target server in the write transaction. Preserve PKCE, exact redirect matching, refresh reuse revocation, and fail-closed storage errors.

Run the oauth2_lifecycle integration target and related authenticated route tests.
