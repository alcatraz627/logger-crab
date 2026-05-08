//! check_s3 — exercise the same aws-sdk-s3 client logger-crab uses, against
//! the credentials in `./.env`, and print the **full** SDK error on failure.
//!
//! Why this exists: logger-crab's runtime collapses SDK errors via
//! `display_sdk_err` into a single line ("head_bucket failed: service
//! error"), which loses the actual AWS error code and message. The AWS CLI
//! shows real errors. The SDK might fail differently than the CLI for subtle
//! reasons (region resolution, signing, retries, BehaviorVersion). This
//! script bridges that gap by running the SDK directly and dumping the
//! error chain in full so the actual cause is visible.
//!
//! Run from the logger-crab repo root:
//!   cargo run -p log-server --example check_s3

use aws_config::BehaviorVersion;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Pick up credentials from ./.env so this matches what the smoke
    // script and the running app would use locally.
    let _ = dotenvy::dotenv();

    let bucket = std::env::var("S3_LOGS_BUCKET")
        .map_err(|_| anyhow::anyhow!("S3_LOGS_BUCKET not set in .env"))?;
    let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".into());

    println!("\n═══ check_s3 (same SDK config as logger-crab runtime) ═══");
    println!("  bucket  = {bucket}");
    println!("  region  = {region}");
    if let Ok(k) = std::env::var("AWS_ACCESS_KEY_ID") {
        let prefix: String = k.chars().take(8).collect();
        println!("  key id  = {prefix}…");
    } else {
        println!("  key id  = (missing AWS_ACCESS_KEY_ID)");
    }
    println!();

    // Build the SDK config exactly as `S3ColdStore::connect` does.
    let region_provider = aws_config::Region::new(region.clone());
    let aws_cfg = aws_config::defaults(BehaviorVersion::latest())
        .region(region_provider)
        .load()
        .await;
    let client = Client::new(&aws_cfg);

    // ─── Test 1: head_bucket ────────────────────────────────────────────
    println!("─── head_bucket ───");
    match client.head_bucket().bucket(&bucket).send().await {
        Ok(_) => println!("  ✓ OK"),
        Err(e) => {
            println!("  ✗ FAIL");
            print_sdk_error(&e);
            anyhow::bail!("head_bucket failed; not running further tests");
        }
    }
    println!();

    // ─── Test 2: put_object ─────────────────────────────────────────────
    println!("─── put_object ───");
    let key = format!(
        "logger-crab-rust-check/{}.txt",
        chrono::Utc::now().timestamp()
    );
    let body_bytes = b"logger-crab rust SDK round-trip check".to_vec();
    let body = ByteStream::from(body_bytes.clone());
    match client
        .put_object()
        .bucket(&bucket)
        .key(&key)
        .body(body)
        .content_type("text/plain")
        .send()
        .await
    {
        Ok(_) => println!("  ✓ OK — wrote s3://{bucket}/{key}"),
        Err(e) => {
            println!("  ✗ FAIL");
            print_sdk_error(&e);
            anyhow::bail!("put_object failed");
        }
    }
    println!();

    // ─── Test 3: get_object (round-trip the body we just wrote) ────────
    println!("─── get_object ───");
    match client.get_object().bucket(&bucket).key(&key).send().await {
        Ok(out) => {
            let bytes = out.body.collect().await?;
            let bytes = bytes.into_bytes();
            if bytes.as_ref() == body_bytes.as_slice() {
                println!("  ✓ OK — round-trip body matches");
            } else {
                println!("  ✗ FAIL — body mismatch");
                println!("     wrote: {body_bytes:?}");
                println!("     read:  {bytes:?}");
            }
        }
        Err(e) => {
            println!("  ✗ FAIL");
            print_sdk_error(&e);
            anyhow::bail!("get_object failed");
        }
    }
    println!();

    println!("═══ All SDK ops succeeded ═══");
    println!("If logger-crab still reports cold.ok=false, the issue is on");
    println!("the Render side (env-var values differ from local .env).");
    println!();

    Ok(())
}

/// Dump every layer of an SDK error so we see the actual AWS message.
/// `{:#?}` on AWS SDK errors gives multi-line output with the cause chain;
/// we also walk `.source()` for any extra detail.
fn print_sdk_error<E>(e: &E)
where
    E: std::fmt::Display + std::fmt::Debug + std::error::Error,
{
    eprintln!("     Display : {e}");
    eprintln!("     Debug   :");
    for line in format!("{e:#?}").lines() {
        eprintln!("       {line}");
    }
    let mut source: Option<&dyn std::error::Error> = e.source();
    let mut depth = 0;
    while let Some(s) = source {
        depth += 1;
        eprintln!("     cause[{depth}]: {s}");
        source = s.source();
    }
}
