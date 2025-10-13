SET timezone='UTC';

CREATE TABLE IF NOT EXISTS telemetry_raw (
    received_at     TIMESTAMPTZ NOT NULL,
    node_id         UBIGINT   NOT NULL,
    partition_date  DATE GENERATED ALWAYS AS (received_at::DATE),
    inside_temp     REAL,
    heater_on       BOOLEAN,
    PRIMARY KEY (node_id, received_at)
) WITH (
    compression = 'alp', 
    partition_by = partition_date, 
    checkpoint_threshold = 1000
);

CREATE VIEW IF NOT EXISTS v_telemetry_by_minute AS
SELECT
    node_id,
    date_trunc('minute', received_at)AS ts,
    MAX(received_at) AS last_raw,
    COUNT(*) AS num_records,
    MIN(inside_temp) AS inside_temp_min,
    MAX(inside_temp) AS inside_temp_max,
    AVG(inside_temp) AS inside_temp_avg,
    AVG(heater_on::INT)::REAL AS heater_ratio
FROM telemetry_raw
GROUP BY node_id, ts;

CREATE VIEW IF NOT EXISTS v_telemetry_hourly AS
SELECT
    node_id,
    date_trunc('hour', ts) AS bucket_start,
    SUM(num_records) AS num_records,
    MIN(inside_temp_min) AS inside_temp_min,
    MAX(inside_temp_max) AS inside_temp_max,
    SUM(inside_temp_avg * num_records) / NULLIF(SUM(num_records), 0)::REAL AS inside_temp_avg,
    SUM(heater_ratio * num_records) / NULLIF(SUM(num_records), 0) AS heater_ratio
FROM v_telemetry_by_minute
GROUP BY node_id, bucket_start;

CREATE VIEW IF NOT EXISTS v_telemetry_daily AS
SELECT
    node_id,
    date_trunc('day', ts) AS bucket_date,
    SUM(num_records) AS num_records,
    MIN(inside_temp_min) AS inside_temp_min,
    MAX(inside_temp_max) AS inside_temp_max,
    SUM(inside_temp_avg * num_records) / NULLIF(SUM(num_records), 0)::REAL AS inside_temp_avg,
    SUM(heater_ratio * num_records) / NULLIF(SUM(num_records), 0)::REAL AS heater_ratio
FROM v_telemetry_by_minute
GROUP BY node_id, bucket_date;

CREATE VIEW IF NOT EXISTS v_telemetry_weekly AS
SELECT
    node_id,
    date_trunc('week', ts) AS week_monday,
    SUM(num_records) AS num_records,
    MIN(inside_temp_min) AS inside_temp_min,
    MAX(inside_temp_max) AS inside_temp_max,
    SUM(inside_temp_avg * num_records) / NULLIF(SUM(num_records), 0)::REAL AS inside_temp_avg,
    SUM(heater_ratio * num_records) / NULLIF(SUM(num_records), 0)::REAL AS heater_ratio
FROM v_telemetry_by_minute
GROUP BY node_id, week_monday;

CREATE VIEW IF NOT EXISTS v_telemetry_monthly AS
SELECT
    node_id,
    date_trunc('month', ts) AS month_start,
    SUM(num_records) AS num_records,
    MIN(inside_temp_min) AS inside_temp_min,
    MAX(inside_temp_max) AS inside_temp_max,
    SUM(inside_temp_avg * num_records) / NULLIF(SUM(num_records), 0)::REAL AS inside_temp_avg,
    SUM(heater_ratio * num_records) / NULLIF(SUM(num_records), 0)::REAL AS heater_ratio
FROM v_telemetry_by_minute
GROUP BY node_id, month_start;
