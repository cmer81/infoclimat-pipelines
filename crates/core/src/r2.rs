//! Client Cloudflare R2 minimal (upload, delete, list).
//!
//! Calqué sur le pattern utilisé par `infoclimat-om-worker/src/cache.rs` :
//! S3-compatible endpoint en mode path-style, région "auto", credentials
//! statiques fournies par l'environnement.

use std::path::Path;

use anyhow::{Context, Result};
use aws_config::{BehaviorVersion, Region};
use aws_credential_types::Credentials;
use aws_sdk_s3::Client;

pub struct R2Config {
    pub account_id: String,
    pub access_key: String,
    pub secret_key: String,
    pub bucket: String,
}

impl R2Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            account_id: std::env::var("R2_ACCOUNT_ID").context("R2_ACCOUNT_ID")?,
            access_key: std::env::var("R2_ACCESS_KEY").context("R2_ACCESS_KEY")?,
            secret_key: std::env::var("R2_SECRET_KEY").context("R2_SECRET_KEY")?,
            bucket: std::env::var("R2_BUCKET").context("R2_BUCKET")?,
        })
    }
}

pub struct R2Client {
    client: Client,
    bucket: String,
}

impl R2Client {
    pub async fn new(cfg: R2Config) -> Result<Self> {
        let endpoint = format!("https://{}.r2.cloudflarestorage.com", cfg.account_id);
        let credentials = Credentials::new(
            cfg.access_key,
            cfg.secret_key,
            None,
            None,
            "pipeline-core",
        );
        let shared = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new("auto"))
            .credentials_provider(credentials)
            .load()
            .await;
        // R2 ne supporte pas les checksums trailer (CRC32) que le SDK AWS 1.x
        // active par défaut (`WhenSupported`). Sans ce paramètre, R2 répond
        // SignatureDoesNotMatch sur le PUT car le SDK signe `x-amz-checksum-crc32`
        // que R2 calcule différemment.
        let s3_cfg = aws_sdk_s3::config::Builder::from(&shared)
            .endpoint_url(endpoint)
            .force_path_style(true)
            .request_checksum_calculation(
                aws_sdk_s3::config::RequestChecksumCalculation::WhenRequired,
            )
            .response_checksum_validation(
                aws_sdk_s3::config::ResponseChecksumValidation::WhenRequired,
            )
            .build();
        Ok(Self {
            client: Client::from_conf(s3_cfg),
            bucket: cfg.bucket,
        })
    }

    pub async fn upload_file(&self, key: &str, local: &Path) -> Result<()> {
        // Lit en mémoire et passe en `ByteStream::from(Bytes)` plutôt que
        // `ByteStream::from_path` : ce dernier déclenche un upload chunked
        // avec checksum trailer que Cloudflare R2 ne supporte pas toujours
        // (silent 4xx). Pour nos fichiers (< 1 MB) le coût mémoire est nul.
        let data = std::fs::read(local)
            .with_context(|| format!("reading {local:?}"))?;
        let bytes = bytes::Bytes::from(data);
        let body = aws_sdk_s3::primitives::ByteStream::from(bytes);
        match self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(body)
            .content_type("application/octet-stream")
            .cache_control("public, max-age=31536000, immutable")
            .send()
            .await
        {
            Ok(_) => {
                tracing::info!(key, "uploaded to R2");
                Ok(())
            }
            Err(e) => {
                let raw = format!("{e:?}");
                let svc = e.into_service_error();
                let code = svc.meta().code().unwrap_or("?");
                let msg = svc.meta().message().unwrap_or("?");
                Err(anyhow::anyhow!(
                    "put_object bucket={} key={key} code={code} msg={msg} raw={raw}",
                    self.bucket
                ))
            }
        }
    }

    /// Écrit des bytes arbitraires sous `key` avec content-type et cache-control
    /// explicites. Utilisé pour les métadonnées JSON (cache court car mutables).
    pub async fn put_bytes(
        &self,
        key: &str,
        data: Vec<u8>,
        content_type: &str,
        cache_control: &str,
    ) -> Result<()> {
        let body = aws_sdk_s3::primitives::ByteStream::from(bytes::Bytes::from(data));
        match self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(body)
            .content_type(content_type)
            .cache_control(cache_control)
            .send()
            .await
        {
            Ok(_) => {
                tracing::info!(key, "put_bytes to R2");
                Ok(())
            }
            Err(e) => {
                let raw = format!("{e:?}");
                let svc = e.into_service_error();
                let code = svc.meta().code().unwrap_or("?");
                let msg = svc.meta().message().unwrap_or("?");
                Err(anyhow::anyhow!(
                    "put_object bucket={} key={key} code={code} msg={msg} raw={raw}",
                    self.bucket
                ))
            }
        }
    }

    pub async fn delete(&self, key: &str) -> Result<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .with_context(|| format!("delete {key}"))?;
        tracing::info!(key, "deleted from R2");
        Ok(())
    }

    pub async fn list_prefix(&self, prefix: &str) -> Result<Vec<String>> {
        let mut keys = Vec::new();
        let mut continuation: Option<String> = None;
        loop {
            let mut req = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(prefix);
            if let Some(t) = &continuation {
                req = req.continuation_token(t);
            }
            let resp = req.send().await?;
            if let Some(contents) = resp.contents {
                for o in contents {
                    if let Some(k) = o.key {
                        keys.push(k);
                    }
                }
            }
            if resp.is_truncated.unwrap_or(false) {
                continuation = resp.next_continuation_token;
            } else {
                break;
            }
        }
        Ok(keys)
    }
}
