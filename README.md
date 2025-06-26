# ETL Job with Rust, Polars, and Shuttle

This repository demonstrates how to build and deploy an ETL (Extract, Transform, Load) job using Rust, Polars, and Shuttle. The service runs on a timer, processes Parquet files from S3 storage, and stores the results in a PostgreSQL database.

## Architecture

```
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│   S3 Bucket     │    │  Shuttle ETL     │    │   PostgreSQL    │
│                 │    │    Service       │    │   Database      │
│ *.parquet files │───▶│                  │───▶│                 │
│ (last 24h)      │    │ - Timer trigger  │    │ ETL results     │
│                 │    │ - Polars proc.   │    │ table           │
└─────────────────┘    │ - Row counting   │    └─────────────────┘
                       └──────────────────┘
```

## Features

- **Scheduled ETL Jobs**: Runs automatically every 24 hours (configurable)
- **S3 Integration**: Uses OpenDAL to access S3-compatible storage
- **Data Processing**: Leverages Polars for efficient file processing
- **Database Storage**: Stores results in PostgreSQL
- **Monitoring**: Health check and status endpoints
- **Infrastructure from Code**: All resources provisioned through Shuttle

## Prerequisites

1. **Shuttle Account**: Create an account at [shuttle.dev](https://shuttle.dev)
2. **Shuttle CLI**: Install following the [installation guide](https://docs.shuttle.dev/getting-started/installation)
3. **S3 Bucket**: An S3-compatible bucket with some Parquet files

## Setup

### 1. Clone and Install Dependencies

```bash
git clone <this-repo>
cd shuttle-etl
```

Dependencies are already configured in `Cargo.toml`.

### 2. Configure S3 Access

Edit `Secrets.toml` with your S3 credentials:

```toml
bucket = "your-s3-bucket-name"
access_key_id = "your-access-key-id"
secret_access_key = "your-secret-access-key"
region = "us-east-1"
```

### 3. Run Locally

```bash
shuttle run
```

This will:

- Start a local PostgreSQL database
- Run migrations to create the `etl_results` table
- Start the ETL timer (runs every 5 minutes in demo mode)
- Serve HTTP endpoints at `http://localhost:8000`

### 4. Test the Endpoints

```bash
# Check service status
curl http://localhost:8000/

# Health check
curl http://localhost:8000/health

# View ETL status and results
curl http://localhost:8000/status
```

## Deployment

Deploy to Shuttle's cloud:

```bash
shuttle deploy
```

## How It Works

### Timer-Based Execution

The service runs an ETL job on a configurable interval:

```rust
// Demo: every 5 minutes, Production: every 24 hours
let mut interval = time::interval(Duration::from_secs(5 * 60));
```

### S3 File Discovery

Scans the S3 bucket for Parquet files modified in the last 24 hours:

```rust
async fn get_recent_parquet_files(storage: &Operator) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let cutoff_time = Utc::now() - ChronoDuration::hours(24);
    // List and filter files...
}
```

### Data Processing with Polars

Each Parquet file is processed using Polars:

```rust
async fn process_parquet_file(storage: &Operator, pool: &PgPool, file_path: &str) -> Result<i64, Box<dyn std::error::Error>> {
    let file_data = storage.read(file_path).await?;
    let cursor = std::io::Cursor::new(file_data.to_vec());
    let df = LazyFrame::scan_parquet(cursor, Default::default())?.collect()?;
    let row_count = df.height() as i64;
    // Store results in database...
}
```

### Database Storage

Results are stored in PostgreSQL with the following schema:

```sql
CREATE TABLE etl_results (
    id SERIAL PRIMARY KEY,
    timestamp TIMESTAMP WITH TIME ZONE NOT NULL,
    file_name VARCHAR NOT NULL,
    row_count BIGINT NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);
```

## Customization

### Add Custom Processing Logic

Replace the simple row counting with your own Polars transformations:

```rust
// Instead of just counting rows:
let row_count = df.height() as i64;

// Add custom processing:
let processed_df = df
    .filter(col("status").eq(lit("active")))
    .group_by([col("category")])
    .agg([col("amount").sum()])
    .sort("amount", Default::default());

let summary_stats = processed_df.collect()?;
```

### Change Timer Frequency

Modify the interval in `etl_timer_loop()`:

```rust
// Every hour
let mut interval = time::interval(Duration::from_secs(60 * 60));

// Daily at midnight (use a cron library for precise scheduling)
let mut interval = time::interval(Duration::from_secs(24 * 60 * 60));
```

### Add Error Handling and Retry Logic

Enhance error handling for production use:

```rust
use tokio::time::{sleep, Duration};

async fn process_with_retry(storage: &Operator, pool: &PgPool, file_path: &str, max_retries: u32) -> Result<i64, Box<dyn std::error::Error>> {
    for attempt in 1..=max_retries {
        match process_parquet_file(storage, pool, file_path).await {
            Ok(result) => return Ok(result),
            Err(e) if attempt == max_retries => return Err(e),
            Err(e) => {
                eprintln!("Attempt {} failed for {}: {}. Retrying...", attempt, file_path, e);
                sleep(Duration::from_secs(attempt as u64 * 2)).await; // Exponential backoff
            }
        }
    }
    unreachable!()
}
```

## Why Rust and Polars?

### Rust Advantages for ETL

- **Zero-cost abstractions**: Lightweight, efficient artifacts
- **Memory safety**: No runtime errors from memory management
- **Static typing**: Catch data validation errors at compile time
- **Performance**: Near C-level performance for data processing
- **Excellent error handling**: Explicit error management with `Result<T, E>`

### Polars Benefits

- **Blazing fast**: Built from the ground up for performance
- **Lazy evaluation**: Optimizes query plans automatically
- **Arrow-based**: Efficient columnar memory format
- **Native Rust**: No Python/C++ interop overhead
- **Rich API**: Comprehensive DataFrame operations

### Shuttle Simplicity

- **Infrastructure from Code**: Resources defined in your application code
- **Zero DevOps**: No manual database setup, networking, or deployment configuration
- **Instant deployment**: From `shuttle deploy` to production-ready service

## Project Structure

```
shuttle-etl/
├── Cargo.toml              # Dependencies and project config
├── Shuttle.toml            # Shuttle deployment configuration
├── Secrets.toml            # S3 credentials (gitignored)
├── migrations/             # Database schema migrations
│   └── 001_create_etl_results.sql
├── src/
│   └── main.rs            # Main application code
└── README.md              # This file
```

## Troubleshooting

### Local Development Issues

1. **Database connection errors**: Ensure Docker is running for local PostgreSQL
2. **S3 access denied**: Verify your credentials in `Secrets.toml`
3. **Polars errors**: Check that your Parquet files are valid

### Deployment Issues

1. **Build failures**: Check that all dependencies are compatible
2. **Runtime errors**: Review logs with `shuttle logs`
3. **Resource provisioning**: Ensure your Shuttle account has the necessary permissions

## Possible Next Steps

- Add monitoring and alerting with metrics collection
- Implement more sophisticated scheduling with cron expressions
- Add data quality checks and validation rules
- Scale processing for larger datasets with parallel execution
- Add support for multiple file formats (CSV, JSON, Arrow)

## Resources

- [Shuttle Documentation](https://docs.shuttle.dev)
- [Polars User Guide](https://pola-rs.github.io/polars/)
- [OpenDAL Documentation](https://opendal.apache.org/)
- [SQLx Documentation](https://docs.rs/sqlx/)
