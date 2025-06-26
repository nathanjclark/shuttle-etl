use axum::{extract::State, routing::get, Json, Router};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use futures_util::stream::TryStreamExt;
use opendal::Operator;
use polars::prelude::*;
use serde::Serialize;
use shuttle_runtime::SecretStore;
use sqlx::PgPool;
use std::time::Duration;
use tempfile::NamedTempFile;
use tokio::time;

#[shuttle_runtime::main]
async fn main(
    #[shuttle_opendal::Opendal(scheme = "s3")] storage: Operator,
    #[shuttle_shared_db::Postgres] pool: PgPool,
    #[shuttle_runtime::Secrets] _secrets: SecretStore,
) -> shuttle_axum::ShuttleAxum {
    // Run database migrations
    sqlx::migrate!()
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    // Start the ETL timer task
    let storage_clone = storage.clone();
    let pool_clone = pool.clone();
    tokio::spawn(async move {
        etl_timer_loop(storage_clone, pool_clone).await;
    });

    // Create router with health check and status endpoints
    let router = Router::new()
        .route("/", get(|| async { "ETL Service Running" }))
        .route("/health", get(health_check))
        .route("/status", get(etl_status))
        .with_state(pool);

    Ok(router.into())
}

async fn etl_timer_loop(storage: Operator, pool: PgPool) {
    // For demo purposes, run every 5 minutes instead of 24 hours
    // Change this to Duration::from_secs(24 * 60 * 60) for production
    let mut interval = time::interval(Duration::from_secs(5 * 60));

    loop {
        interval.tick().await;
        println!("Starting ETL job at {}", Utc::now());

        if let Err(e) = run_etl_job(&storage, &pool).await {
            eprintln!("ETL job failed: {}", e);
        } else {
            println!("ETL job completed successfully");
        }
    }
}

async fn run_etl_job(storage: &Operator, pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting ETL job...");

    // Get files from last 24 hours
    let recent_files = get_recent_parquet_files(storage).await?;
    println!("Found {} recent parquet files", recent_files.len());

    // Process each file
    for file_path in recent_files {
        match process_parquet_file(storage, pool, &file_path).await {
            Ok(row_count) => {
                println!("Processed {}: {} rows", file_path, row_count);
            }
            Err(e) => {
                eprintln!("Failed to process {}: {}", file_path, e);
            }
        }
    }

    Ok(())
}

async fn get_recent_parquet_files(
    storage: &Operator,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut recent_files = Vec::new();
    let cutoff_time = Utc::now() - ChronoDuration::hours(24);

    // List all objects in the bucket
    let mut lister = storage.lister("/").await?;

    while let Some(entry) = lister.try_next().await? {
        let path = entry.path();

        // Filter for .parquet files
        if path.ends_with(".parquet") {
            // Get metadata to check modification time
            let metadata = storage.stat(path).await?;

            if let Some(modified) = metadata.last_modified() {
                if modified > cutoff_time {
                    recent_files.push(path.to_string());
                }
            }
        }
    }

    Ok(recent_files)
}

async fn process_parquet_file(
    storage: &Operator,
    pool: &PgPool,
    file_path: &str,
) -> Result<i64, Box<dyn std::error::Error>> {
    // Download file from S3
    let file_data = storage.read(file_path).await?;

    // Create a temporary file for Polars to read from
    let mut temp_file = NamedTempFile::new()?;
    std::io::Write::write_all(&mut temp_file, &file_data.to_bytes())?;

    // Read parquet file into Polars DataFrame
    let df = LazyFrame::scan_parquet(temp_file.path(), Default::default())?.collect()?;

    // Count rows (this is where users can add their own logic)
    let row_count = df.height() as i64;

    // Store results in database
    store_etl_result(pool, file_path, row_count).await?;

    Ok(row_count)
}

async fn store_etl_result(
    pool: &PgPool,
    file_name: &str,
    row_count: i64,
) -> Result<(), sqlx::Error> {
    let now = Utc::now();
    sqlx::query("INSERT INTO etl_results (timestamp, file_name, row_count) VALUES ($1, $2, $3)")
        .bind(now)
        .bind(file_name)
        .bind(row_count)
        .execute(pool)
        .await?;

    Ok(())
}

async fn health_check() -> &'static str {
    "OK"
}

#[derive(Serialize)]
struct EtlStatus {
    last_run: Option<DateTime<Utc>>,
    total_files_processed: i64,
    service_status: String,
}

async fn etl_status(State(pool): State<PgPool>) -> Json<EtlStatus> {
    let last_run: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT MAX(timestamp) FROM etl_results")
            .fetch_optional(&pool)
            .await
            .unwrap_or(None)
            .flatten();

    let total_files: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM etl_results")
        .fetch_one(&pool)
        .await
        .unwrap_or(0);

    Json(EtlStatus {
        last_run,
        total_files_processed: total_files,
        service_status: "running".to_string(),
    })
}
