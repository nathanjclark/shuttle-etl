CREATE TABLE IF NOT EXISTS etl_results (
    id SERIAL PRIMARY KEY,
    timestamp TIMESTAMP WITH TIME ZONE NOT NULL,
    file_name VARCHAR NOT NULL,
    row_count BIGINT NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_etl_results_timestamp ON etl_results(timestamp);
CREATE INDEX idx_etl_results_file_name ON etl_results(file_name); 