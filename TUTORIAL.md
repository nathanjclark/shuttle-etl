# How to Build an ETL Job with Rust, Polars and Shuttle

Data processing doesn't have to be complex or resource-heavy. In this tutorial, you'll learn how to build a simple ETL (Extract, Transform, Load) job using Rust, Polars, and Shuttle that automatically processes parquet files from S3 and stores results in PostgreSQL - all with minimal infrastructure setup.

## Why Use Rust and Polars for ETL?

When building data pipelines, the choice of technology stack can make or break your project's success. Traditional ETL solutions often force you to choose between performance and simplicity, but Rust changes this equation entirely.

Rust excels at ETL workloads because of its zero-cost abstractions and sophisticated memory management. While Python-based solutions like pandas can consume gigabytes of RAM processing large datasets, Rust applications typically use a fraction of that memory while running significantly faster. This efficiency translates directly to lower cloud costs and better resource utilization in production environments.

The language's borrow checker and static typing system provide another crucial advantage: they catch data processing errors at compile time rather than during runtime. When you're processing financial transactions, user data, or other critical information, discovering a bug during deployment rather than at 3 AM on a weekend can save both money and reputation. Rust ensures your ETL jobs are robust before they ever touch production data.

[Polars](https://pola.rs/) represents the next generation of DataFrame libraries, built from the ground up in Rust for maximum performance. Unlike Python environments where you're bridging between pandas and Polars, using Polars natively eliminates the overhead of language boundaries and foreign function interfaces. Your data transformations run at native speed without the complexity of managing multiple runtime environments.

Perhaps most importantly, Rust's ownership model makes memory leaks and data races nearly impossible. For ETL jobs that need to run reliably for months or years, processing terabytes of data, this reliability is invaluable. Combined with Shuttle's infrastructure automation, you get enterprise-grade performance with startup-friendly operational simplicity.

## Building the ETL Pipeline

Our ETL job will follow this architecture:

```
┌─────────────┐    ┌──────────────────┐    ┌─────────────┐
│   S3 Bucket │───▶│  Shuttle ETL     │───▶│ PostgreSQL  │
│ (*.parquet) │    │  Service         │    │ Database    │
│             │    │ (Daily Timer)    │    │             │
└─────────────┘    └──────────────────┘    └─────────────┘
```

### Prerequisites

Before we dive into the implementation, you'll need to set up a few things. First, create a free [Shuttle account](https://console.shuttle.dev) if you haven't already. Shuttle's community tier is generous enough for development, making it perfect for getting started with Rust-based ETL jobs.

Next, install the [Shuttle CLI](https://docs.shuttle.dev/getting-started/installation) on your development machine. The CLI handles project initialization, local development, and deployment with simple commands that abstract away the complexity of cloud infrastructure management.

Finally, gather your S3 credentials. You'll need an access key ID, secret access key, bucket name, and region. While we're using AWS S3 in this tutorial, Shuttle's OpenDAL integration works with any S3-compatible storage service, including MinIO, DigitalOcean Spaces, or Cloudflare R2.

### Step 1: Initialize Your Project

Let's start by creating a new Shuttle project. The Shuttle CLI makes this process straightforward, generating a basic project structure with all the necessary configuration files.

```bash
shuttle init shuttle-etl
cd shuttle-etl
```

This command creates a new directory with a basic Rust project structure optimized for Shuttle deployment. You'll notice it includes a `Cargo.toml` file, a `src/main.rs` with a simple example, and a `.gitignore` file that's already configured to exclude sensitive files like `Secrets.toml`.

### Step 2: Set Up Dependencies

Now we need to configure our project's dependencies to include everything required for our ETL pipeline. Open your `Cargo.toml` file and replace the dependencies section with the following configuration:

```toml
[package]
name = "shuttle-etl"
version = "0.1.0"
edition = "2021"

[dependencies]
axum = "0.8.4"
shuttle-runtime = "0.55.0"
shuttle-axum = "0.55.0"
shuttle-opendal = "0.55.0"
shuttle-shared-db = { version = "0.55.0", features = ["postgres", "sqlx"] }
tokio = { version = "1.0", features = ["full"] }
polars = { version = "0.38", features = ["lazy", "parquet"] }
chrono = { version = "0.4", features = ["serde"] }
sqlx = { version = "0.8", features = ["postgres", "runtime-tokio-rustls", "chrono"] }
opendal = "0.51"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
futures-util = "0.3"
tempfile = "3.0"
```

Each of these dependencies serves a specific purpose in our ETL pipeline. The Shuttle-specific crates handle infrastructure provisioning, while Polars provides high-performance data processing capabilities. SQLx gives us type-safe database interactions, and the remaining crates handle various utility functions like JSON serialization and temporary file management.

### Step 3: Database Schema

Database migrations are a critical part of any data pipeline, ensuring your schema stays consistent across development, staging, and production environments. Shuttle supports SQLx migrations out of the box, so let's set up our database schema.

First, create a directory to hold our migration files:

```bash
mkdir migrations
```

Now create our initial migration file at `migrations/001_create_etl_results.sql`:

```sql
CREATE TABLE IF NOT EXISTS etl_results (
    id SERIAL PRIMARY KEY,
    timestamp TIMESTAMP WITH TIME ZONE NOT NULL,
    file_name VARCHAR NOT NULL,
    row_count BIGINT NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_etl_results_timestamp ON etl_results(timestamp);
CREATE INDEX idx_etl_results_file_name ON etl_results(file_name);
```

This schema defines a simple but effective structure for tracking our ETL job results. The `timestamp` field records when each file was processed, while `file_name` and `row_count` store the essential results of our processing. The indexes ensure fast queries when monitoring job performance or debugging issues. The `created_at` field provides an audit trail of when records were inserted into the database.

### Step 4: Configure Secrets

Shuttle provides a secure way to handle secrets that keeps them out of your code repository while making them available to your application at runtime.

Create a `Secrets.toml` file in your project root for your S3 credentials:

```toml
# S3 Configuration
S3_BUCKET = "your-bucket-name"
S3_ACCESS_KEY_ID = "your-access-key"
S3_SECRET_ACCESS_KEY = "your-secret-key"
S3_REGION = "us-east-1"
```

Remember to add `Secrets.toml` to your `.gitignore` file to prevent accidentally committing sensitive credentials to version control. Shuttle automatically excludes this file from deployment archives while making the secrets available to your application through environment variables.

You'll also need to configure Shuttle to include your migration files in the deployment package. Create or update your `Shuttle.toml` file:

```toml
assets = ["migrations"]
```

This tells Shuttle to include the migrations directory when building your deployment package, ensuring your database schema is properly set up in production.

### Step 5: Implement the ETL Logic

Now comes the heart of our application. We'll implement the main ETL logic that ties together all our components: the timer, S3 access, data processing, and database storage. Open `src/main.rs` and replace its contents with our implementation:

```rust
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
```

This main function demonstrates the power of Shuttle's "Infrastructure as Code" approach. The `#[shuttle_opendal::Opendal(scheme = "s3")]` annotation automatically provisions S3 access using the credentials from your `Secrets.toml` file, while `#[shuttle_shared_db::Postgres]` creates a PostgreSQL database instance and provides a connection pool. No manual infrastructure setup required.

The function handles three key responsibilities: running database migrations to ensure schema consistency, spawning the ETL timer as a background task, and setting up HTTP endpoints for monitoring and health checks. This design allows your ETL job to run continuously while providing visibility into its operation.

### Step 6: Timer and Job Orchestration

```rust
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
```

The timer system forms the backbone of our ETL pipeline. The `etl_timer_loop` function creates a reliable scheduler that runs our job at regular intervals. For development and testing, we've set it to run every 5 minutes, but you can easily change this to 24 hours for production use by modifying the `Duration::from_secs` parameter.

Notice how the error handling works here. If an individual ETL job fails, we log the error but continue with the next scheduled run. This resilience is crucial for production systems where temporary failures shouldn't stop the entire pipeline. The `run_etl_job` function orchestrates the complete ETL process: discovering new files, processing each one individually, and handling errors gracefully.

### Step 7: S3 File Discovery

```rust
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
```

The file discovery mechanism is where our ETL pipeline determines what work needs to be done. This function demonstrates several important concepts for working with object storage at scale. First, we establish a cutoff time of 24 hours ago, ensuring we only process recently added files and avoid reprocessing old data.

The streaming approach using `lister.try_next()` is crucial for performance when dealing with buckets containing thousands or millions of objects. Instead of loading all object metadata into memory at once, we process objects one at a time, keeping memory usage constant regardless of bucket size. For each object, we check both the filename extension and the modification timestamp, giving us precise control over which files get processed.

### Step 8: Polars Data Processing

```rust
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
```

Here's where the real data processing magic happens. This function showcases Polars' power while demonstrating how to handle the practical challenges of processing files from object storage. Since Polars' parquet reader expects a file path rather than raw bytes, we use a temporary file as an intermediate step. This approach is both memory-efficient and performant, even for large files.

The key insight is in the data processing workflow: download the file, create a temporary local copy, process it with Polars, extract the results, and clean up. This pattern works well for batch processing scenarios and can be easily extended to handle more complex transformations.

The simple row count we're performing here is just the beginning. You can replace this basic operation with sophisticated data transformations:

```rust
// Example: Calculate statistics
let stats = df
    .lazy()
    .select([
        col("*").count().alias("total_rows"),
        col("amount").sum().alias("total_amount"),
        col("amount").mean().alias("avg_amount"),
    ])
    .collect()?;

// Example: Filter and aggregate
let processed = df
    .lazy()
    .filter(col("status").eq(lit("active")))
    .group_by([col("category")])
    .agg([col("value").sum().alias("category_total")])
    .collect()?;
```

### Step 9: Database Storage

```rust
async fn store_etl_result(
    pool: &PgPool,
    file_name: &str,
    row_count: i64,
) -> Result<(), sqlx::Error> {
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO etl_results (timestamp, file_name, row_count) VALUES ($1, $2, $3)"
    )
    .bind(now)
    .bind(file_name)
    .bind(row_count)
    .execute(pool)
    .await?;

    Ok(())
}
```

Database storage represents the final step in our ETL pipeline, where we persist the results of our data processing. This function demonstrates a straightforward but robust approach to data persistence. By using parameterized queries with SQLx, we get the safety benefits of prepared statements while keeping the code simple and readable.

The choice to avoid ORMs here is deliberate. ETL jobs often involve custom queries, bulk operations, and performance-critical database interactions where the simplicity and directness of SQL provides better control and transparency than object-relational mapping layers.

### Step 10: Monitoring Endpoints

```rust
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
    let last_run: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT MAX(timestamp) FROM etl_results"
    )
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
```

Monitoring and observability are essential for any production ETL system. These endpoints provide real-time insight into your pipeline's health and performance. The health check endpoint offers a simple way for load balancers and monitoring systems to verify your service is running, while the status endpoint provides more detailed operational metrics.

## Deployment & Conclusion

### Deploy to Shuttle

One of Shuttle's greatest strengths is how it eliminates the complexity traditionally associated with deploying data infrastructure. What would normally require orchestrating multiple cloud services, managing environment variables, and configuring networking becomes a simple two-command process:

```bash
# Test locally first
shuttle run

# Deploy to production
shuttle deploy
```

Behind these simple commands, Shuttle handles an enormous amount of infrastructure complexity. It provisions your PostgreSQL database with proper connection pooling and backup configuration, sets up S3 access using your credentials with appropriate security policies, handles HTTPS certificates and load balancing for your HTTP endpoints, runs your database migrations to ensure schema consistency, and starts your timer-based ETL job as a long-running service.

### Monitor Your ETL Job

The HTTP endpoints we created give you real-time status information that you can access from any browser or integrate with monitoring tools. Visit your deployed service's `/health` endpoint for a simple health check, or check `/status` for detailed operational metrics including when the job last ran and how many files have been processed.

For deeper troubleshooting, the `shuttle logs` command provides access to your application's complete log output, including the detailed processing information we added with `println!` statements throughout our code. These logs can also be accessed via the Shuttle web console.

### Next Steps & Customization

The ETL pipeline we've built provides a solid foundation that can be extended for sophisticated data processing scenarios. For more complex data transformations, Polars offers a rich API that includes joins, window functions, complex aggregations, and advanced DataFrame operations. The [Polars documentation](https://docs.pola.rs/) provides comprehensive examples for these advanced features.

Production ETL systems often require more robust error handling than our basic implementation provides. Consider adding retry logic using crates like `backoff`, implementing dead letter queues for failed processing, or integrating alerting systems using the `notify` crate to send notifications when jobs fail or take longer than expected.

Data quality is another crucial consideration for production pipelines. Rust's type system already provides significant safety guarantees, but you can enhance this further by integrating validation crates like `validator` to ensure incoming data meets your quality standards before processing.

As your data architecture grows, you might want to integrate multiple data sources. Shuttle supports numerous database and storage integrations including `shuttle-turso` for SQLite, `shuttle-qdrant` for vector databases, and various other specialized data stores. Each integration follows the same annotation-based pattern we've used here.

If you prefer working with ORMs rather than direct SQL, you can easily integrate `SeaORM` or `Diesel` into this foundation. These tools provide type-safe database interactions with more sophisticated relationship modeling capabilities.

For more sophisticated scheduling requirements, consider replacing our simple timer with `tokio-cron-scheduler`, which provides cron-like scheduling syntax and more flexible timing options.

### Why This Approach Wins

Traditional ETL infrastructure deployment is notoriously complex, typically requiring expertise in container orchestration platforms like Kubernetes or Docker Swarm, manual database provisioning and migration management, secure secret distribution systems, load balancer configuration and SSL certificate management, and comprehensive monitoring and logging infrastructure setup.

With Shuttle, all of this operational complexity disappears behind simple annotations in your code. You focus exclusively on your business logic while Shuttle handles the infrastructure concerns automatically. This represents true "Infrastructure from Code" where your application requirements directly drive infrastructure provisioning.

The result is an ETL job that delivers great performance and reliability while operating at a fraction of the cost and complexity of traditional cloud deployments.
