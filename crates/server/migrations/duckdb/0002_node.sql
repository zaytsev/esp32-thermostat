CREATE TABLE IF NOT EXISTS node_config (
    node_id UBIGINT PRIMARY KEY,
    name TEXT,
    target_temp REAL,
    temp_tolerance REAL,
);

CREATE TABLE IF NOT EXISTS node_activity_journal (
    node_id         UBIGINT   NOT NULL,
    received_at     TIMESTAMPTZ NOT NULL,
    partition_date  DATE GENERATED ALWAYS AS (received_at::DATE),
    PRIMARY KEY (node_id, received_at)
) WITH (
    compression = 'alp', 
    partition_by = partition_date, 
    checkpoint_threshold = 1000
);
