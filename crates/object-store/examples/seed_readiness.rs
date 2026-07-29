use bytes::Bytes;
use spool_object_store::{ObjectStore, S3Configuration, S3ObjectStore};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store = S3ObjectStore::new(S3Configuration {
        bucket: env::var("SPOOL_S3_BUCKET")?,
        region: env::var("SPOOL_S3_REGION").unwrap_or_else(|_| "auto".into()),
        endpoint: env::var("SPOOL_S3_ENDPOINT").ok(),
        access_key_id: env::var("SPOOL_S3_ACCESS_KEY_ID")?,
        secret_access_key: env::var("SPOOL_S3_SECRET_ACCESS_KEY")?,
        allow_http: false,
        virtual_hosted_style: env::var("SPOOL_S3_VIRTUAL_HOSTED_STYLE").as_deref() == Ok("true"),
    })?;
    store
        .put(
            "health/readiness-probe",
            Bytes::from_static(b"spool-ready"),
            None,
        )
        .await?;
    println!("seeded health/readiness-probe");
    Ok(())
}
