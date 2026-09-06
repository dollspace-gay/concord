//! Authenticated encryption for recoverable external-provider credentials.
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use rand::Rng;
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(serde::Deserialize, serde::Serialize)]
struct StoredEnvelope {
    key_id: String,
    ciphertext: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct RotationReport {
    pub accounts: u64,
    pub signing_keys: u64,
    pub pending_oauth: u64,
}
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MigrationReport {
    pub migrated: u64,
    pub missing_data: u64,
    pub signing_keys: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("external credential key is unavailable")]
    Unavailable,
    #[error("external credential key is malformed")]
    Malformed,
    #[error("external credential cannot be decrypted")]
    Decrypt,
    #[error("external credential encryption failed")]
    Encrypt,
}
#[derive(Clone)]
pub struct SecretVault {
    cipher: XChaCha20Poly1305,
    key_id: String,
}
impl SecretVault {
    pub fn load(path: &Path) -> Result<Self, SecretError> {
        let encoded = std::fs::read_to_string(path).map_err(|_| SecretError::Unavailable)?;
        let bytes = hex::decode(encoded.trim()).map_err(|_| SecretError::Malformed)?;
        if bytes.len() != 32 {
            return Err(SecretError::Malformed);
        }
        let cipher =
            XChaCha20Poly1305::new_from_slice(&bytes).map_err(|_| SecretError::Malformed)?;
        let key_id = hex::encode(&Sha256::digest(&bytes)[..8]);
        Ok(Self { cipher, key_id })
    }
    pub fn key_id(&self) -> &str {
        &self.key_id
    }
    pub fn encrypt(&self, context: &str, plaintext: &[u8]) -> Result<String, SecretError> {
        let mut nonce = [0u8; 24];
        rand::rng().fill_bytes(&mut nonce);
        let ciphertext = self
            .cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: plaintext,
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| SecretError::Encrypt)?;
        let mut envelope = Vec::with_capacity(24 + ciphertext.len());
        envelope.extend_from_slice(&nonce);
        envelope.extend_from_slice(&ciphertext);
        Ok(URL_SAFE_NO_PAD.encode(envelope))
    }
    pub fn decrypt(
        &self,
        context: &str,
        envelope: &str,
        key_id: &str,
    ) -> Result<Vec<u8>, SecretError> {
        if key_id != self.key_id {
            return Err(SecretError::Unavailable);
        }
        let envelope = URL_SAFE_NO_PAD
            .decode(envelope)
            .map_err(|_| SecretError::Decrypt)?;
        if envelope.len() < 40 {
            return Err(SecretError::Decrypt);
        }
        self.cipher
            .decrypt(
                <&XNonce>::try_from(&envelope[..24]).map_err(|_| SecretError::Decrypt)?,
                Payload {
                    msg: &envelope[24..],
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| SecretError::Decrypt)
    }
}

pub async fn rotate_external_envelopes(
    pool: &sqlx::SqlitePool,
    old: &SecretVault,
    new: &SecretVault,
) -> Result<RotationReport, anyhow::Error> {
    if old.key_id() == new.key_id() {
        anyhow::bail!("new external credential key must differ from the current key");
    }
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let accounts:Vec<(String,String,String)>=sqlx::query_as("SELECT user_id,credential_key_id,credential_ciphertext FROM oauth_accounts WHERE provider='atproto' AND credential_state='active'").fetch_all(&mut *tx).await?;
    for (user_id, key_id, ciphertext) in &accounts {
        let context = format!("atproto:{user_id}");
        let plaintext = old.decrypt(&context, ciphertext, key_id)?;
        let replacement = new.encrypt(&context, &plaintext)?;
        let changed=sqlx::query("UPDATE oauth_accounts SET credential_key_id=?,credential_ciphertext=?,credential_version=credential_version+1 WHERE user_id=? AND provider='atproto' AND credential_key_id=?")
            .bind(new.key_id()).bind(replacement).bind(user_id).bind(key_id).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            anyhow::bail!("external account credential changed during rotation");
        }
    }
    let signing: Option<String> =
        sqlx::query_scalar("SELECT value FROM server_config WHERE key='atproto_signing_key'")
            .fetch_optional(&mut *tx)
            .await?;
    let mut signing_count = 0;
    if let Some(value) = signing {
        let envelope: StoredEnvelope = serde_json::from_str(&value).map_err(|_| {
            anyhow::anyhow!("AT signing key is not encrypted; run external secret migration first")
        })?;
        let plaintext = old.decrypt(
            "atproto:client-signing-key",
            &envelope.ciphertext,
            &envelope.key_id,
        )?;
        let replacement = StoredEnvelope {
            key_id: new.key_id().into(),
            ciphertext: new.encrypt("atproto:client-signing-key", &plaintext)?,
        };
        sqlx::query("UPDATE server_config SET value=?,updated_at=datetime('now') WHERE key='atproto_signing_key'").bind(serde_json::to_string(&replacement)?).execute(&mut *tx).await?;
        signing_count = 1;
    }
    let pending:Vec<(String,String,String)>=sqlx::query_as("SELECT state_hash,credential_key_id,credential_ciphertext FROM pending_atproto_oauth WHERE state='pending'").fetch_all(&mut *tx).await?;
    for (state_hash, key_id, ciphertext) in &pending {
        let context = format!("atproto:pending:{state_hash}");
        let plaintext = old.decrypt(&context, ciphertext, key_id)?;
        let replacement = new.encrypt(&context, &plaintext)?;
        let changed=sqlx::query("UPDATE pending_atproto_oauth SET credential_key_id=?,credential_ciphertext=? WHERE state_hash=? AND state='pending' AND credential_key_id=?")
            .bind(new.key_id()).bind(replacement).bind(state_hash).bind(key_id).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            anyhow::bail!("pending OAuth state changed during rotation");
        }
    }
    let advanced = sqlx::query(
        "UPDATE credential_rotation_state
         SET phase='database_committed',updated_at=datetime('now')
         WHERE singleton=1 AND old_key_id=? AND new_key_id=? AND phase='prepared'",
    )
    .bind(old.key_id())
    .bind(new.key_id())
    .execute(&mut *tx)
    .await?;
    if advanced.rows_affected() != 1 {
        anyhow::bail!("credential rotation was not durably prepared");
    }
    tx.commit().await?;
    Ok(RotationReport {
        accounts: accounts.len() as u64,
        signing_keys: signing_count,
        pending_oauth: pending.len() as u64,
    })
}

pub async fn migrate_legacy_atproto_credentials(
    pool: &sqlx::SqlitePool,
    vault: &SecretVault,
) -> Result<MigrationReport, anyhow::Error> {
    type LegacyRow = (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let rows:Vec<LegacyRow>=sqlx::query_as("SELECT user_id,provider_id,access_token,refresh_token,dpop_private_key,pds_url,token_expires_at FROM oauth_accounts WHERE provider='atproto' AND credential_state='legacy_plaintext'").fetch_all(&mut *tx).await?;
    let mut report = MigrationReport::default();
    for (user_id, did, access, refresh, dpop, pds, expires) in rows {
        let (
            Some(access_token),
            Some(refresh_token),
            Some(dpop_private_key),
            Some(pds_url),
            Some(token_expires_at),
        ) = (access, refresh, dpop, pds, expires)
        else {
            sqlx::query("UPDATE oauth_accounts SET credential_state='missing_data' WHERE user_id=? AND provider='atproto' AND credential_state='legacy_plaintext'").bind(&user_id).execute(&mut *tx).await?;
            report.missing_data += 1;
            continue;
        };
        let credentials = crate::db::queries::users::AtprotoCredentials {
            did,
            access_token,
            refresh_token,
            dpop_private_key,
            pds_url,
            authorization_issuer: String::new(),
            token_endpoint: String::new(),
            token_expires_at,
            credential_version: 0,
        };
        let context = format!("atproto:{user_id}");
        let ciphertext = vault.encrypt(&context, &serde_json::to_vec(&credentials)?)?;
        let changed=sqlx::query("UPDATE oauth_accounts SET credential_key_id=?,credential_ciphertext=?,credential_version=credential_version+1,credential_state='active',access_token=NULL,refresh_token=NULL,dpop_private_key=NULL WHERE user_id=? AND provider='atproto' AND credential_state='legacy_plaintext'")
            .bind(vault.key_id()).bind(ciphertext).bind(&user_id).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            anyhow::bail!("legacy credential changed during migration");
        }
        report.migrated += 1;
    }
    if let Some(stored) = sqlx::query_scalar::<_, String>(
        "SELECT value FROM server_config WHERE key='atproto_signing_key'",
    )
    .fetch_optional(&mut *tx)
    .await?
    .filter(|stored| serde_json::from_str::<StoredEnvelope>(stored).is_err())
    {
        let envelope = StoredEnvelope {
            key_id: vault.key_id().into(),
            ciphertext: vault.encrypt("atproto:client-signing-key", stored.as_bytes())?,
        };
        let changed = sqlx::query("UPDATE server_config SET value=?,updated_at=datetime('now') WHERE key='atproto_signing_key' AND value=?")
                .bind(serde_json::to_string(&envelope)?)
                .bind(&stored)
                .execute(&mut *tx)
                .await?;
        if changed.rows_affected() != 1 {
            anyhow::bail!("AT signing key changed during migration");
        }
        report.signing_keys = 1;
    }
    tx.commit().await?;
    Ok(report)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn decrypts_envelope_from_chacha20poly1305_0_10() {
        // Generated with chacha20poly1305 0.10.1, key [7; 32], nonce [9; 24].
        let key_file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(key_file.path(), hex::encode([7_u8; 32])).unwrap();
        let vault = SecretVault::load(key_file.path()).unwrap();
        let envelope =
            "CQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJ0heF6x_TVBKGfX5VDBZ1xl6c3P8umTS9dFTqmu9gIX5dcIRADA";
        assert_eq!(
            vault
                .decrypt("oauth:fixture", envelope, vault.key_id())
                .unwrap(),
            b"legacy-provider-token"
        );
        assert!(
            vault
                .decrypt("oauth:other", envelope, vault.key_id())
                .is_err()
        );
    }

    #[test]
    fn round_trip_is_randomized_and_context_bound() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, "{}", hex::encode([7u8; 32])).unwrap();
        let v = SecretVault::load(f.path()).unwrap();
        let a = v.encrypt("account:a", b"token").unwrap();
        let b = v.encrypt("account:a", b"token").unwrap();
        assert_ne!(a, b);
        assert_eq!(v.decrypt("account:a", &a, v.key_id()).unwrap(), b"token");
        assert!(v.decrypt("account:b", &a, v.key_id()).is_err());
    }

    #[tokio::test]
    async fn legacy_migration_encrypts_signing_key_and_marks_incomplete_accounts() {
        let pool = crate::db::pool::create_pool("sqlite::memory:")
            .await
            .unwrap();
        crate::db::pool::run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('complete','complete'),('incomplete','incomplete')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO oauth_accounts(user_id,provider,provider_id,access_token,refresh_token,dpop_private_key,pds_url,token_expires_at,credential_state) VALUES('complete','atproto','did:plc:complete','access','refresh','dpop','https://pds.test','2030-01-01T00:00:00Z','legacy_plaintext'),('incomplete','atproto','did:plc:incomplete','access',NULL,'dpop','https://pds.test','2030-01-01T00:00:00Z','legacy_plaintext')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO server_config(key,value) VALUES('atproto_signing_key','legacy-private-jwk')")
            .execute(&pool)
            .await
            .unwrap();
        let mut key = tempfile::NamedTempFile::new().unwrap();
        writeln!(key, "{}", hex::encode([9u8; 32])).unwrap();
        let vault = SecretVault::load(key.path()).unwrap();

        let report = migrate_legacy_atproto_credentials(&pool, &vault)
            .await
            .unwrap();
        assert_eq!(
            report,
            MigrationReport {
                migrated: 1,
                missing_data: 1,
                signing_keys: 1,
            }
        );
        let complete: (String, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT credential_state,access_token,credential_ciphertext FROM oauth_accounts WHERE user_id='complete'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(complete.0, "active");
        assert!(complete.1.is_none());
        assert!(complete.2.is_some());
        let incomplete: String = sqlx::query_scalar(
            "SELECT credential_state FROM oauth_accounts WHERE user_id='incomplete'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(incomplete, "missing_data");
        let signing: String =
            sqlx::query_scalar("SELECT value FROM server_config WHERE key='atproto_signing_key'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let envelope: StoredEnvelope = serde_json::from_str(&signing).unwrap();
        assert_eq!(
            vault
                .decrypt(
                    "atproto:client-signing-key",
                    &envelope.ciphertext,
                    &envelope.key_id,
                )
                .unwrap(),
            b"legacy-private-jwk"
        );
    }
}
