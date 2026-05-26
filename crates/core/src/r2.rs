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
        let s3_cfg = aws_sdk_s3::config::Builder::from(&shared)
            .endpoint_url(endpoint)
            .force_path_style(true)
            .build();
        Ok(Self {
            client: Client::from_conf(s3_cfg),
            bucket: cfg.bucket,
        })
    }

    pub async fn upload_file(&self, key: &str, local: &Path) -> Result<()> {
        let body = aws_sdk_s3::primitives::ByteStream::from_path(local)
            .await
            .with_context(|| format!("reading {local:?}"))?;
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(body)
            .content_type("application/octet-stream")
            .cache_control("public, max-age=31536000, immutable")
            .send()
            .await
            .with_context(|| format!("upload {key}"))?;
        tracing::info!(key, "uploaded to R2");
        Ok(())
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
