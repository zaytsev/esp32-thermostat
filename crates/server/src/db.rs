use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use chrono::{DateTime, Datelike, Duration, DurationRound, TimeZone, Utc};
use duckdb::{Connection, OptionalExt, params};
use esp32_thermostat_common::proto::TelemetryMessage;
use rust_embed::Embed;
use serde::{Deserialize, Serialize};
use tokio::task;
use tracing::{debug, error, info};

use crate::telemetry::TelemetryProcessor;

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub enum Granularity {
    Minute,
    Hour,
    Day,
    Week,
    Month,
}

impl Granularity {
    // which SQL view to query
    fn view(&self) -> &'static str {
        match self {
            Self::Minute => "v_telemetry_by_minute",
            Self::Hour => "v_telemetry_hourly",
            Self::Day => "v_telemetry_daily",
            Self::Week => "v_telemetry_weekly",
            Self::Month => "v_telemetry_monthly",
        }
    }

    // name of the date/time column to filter on
    fn ts_col(&self) -> &'static str {
        match self {
            Self::Minute => "ts",
            Self::Hour => "bucket_start",
            Self::Day => "bucket_date",
            Self::Week => "week_monday",
            Self::Month => "month_start",
        }
    }

    pub fn duration(&self) -> Duration {
        match self {
            Granularity::Minute => Duration::minutes(1),
            Granularity::Hour => Duration::hours(1),
            Granularity::Day => Duration::days(1),
            Granularity::Week => Duration::weeks(1),
            Granularity::Month => Duration::days(30),
        }
    }

    pub fn next(&self) -> Option<Self> {
        match self {
            Granularity::Minute => Some(Granularity::Hour),
            Granularity::Hour => Some(Granularity::Day),
            Granularity::Day => Some(Granularity::Week),
            Granularity::Week => Some(Granularity::Month),
            Granularity::Month => None,
        }
    }

    pub fn floor(&self, ts: DateTime<Utc>) -> DateTime<Utc> {
        match self {
            Granularity::Minute => ts.duration_trunc(self.duration()).unwrap(),
            Granularity::Hour => ts.duration_trunc(self.duration()).unwrap(),
            Granularity::Day => ts.duration_trunc(self.duration()).unwrap(),
            Granularity::Week => {
                // Monday 00:00:00 UTC of the same week
                let days_from_mon = ts.weekday().num_days_from_monday();
                ts.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc()
                    - Duration::days(days_from_mon as i64)
            }
            Granularity::Month => {
                // first day of the month, 00:00:00 UTC
                Utc.with_ymd_and_hms(ts.year(), ts.month(), 1, 0, 0, 0)
                    .unwrap()
            }
        }
    }

    pub fn ceil(&self, ts: DateTime<Utc>) -> DateTime<Utc> {
        let floored = self.floor(ts);
        if floored == ts {
            match self {
                Granularity::Month => {
                    let next = if floored.month() == 12 {
                        Utc.with_ymd_and_hms(floored.year() + 1, 1, 1, 0, 0, 0)
                            .unwrap()
                    } else {
                        Utc.with_ymd_and_hms(floored.year(), floored.month() + 1, 1, 0, 0, 0)
                            .unwrap()
                    };
                    next
                }
                _ => floored + self.duration(),
            }
        } else {
            floored + self.duration()
        }
    }
}

#[derive(Debug, Clone)]
pub struct DataPoint {
    pub ts: DateTime<Utc>,
    pub num_records: i64,
    pub min_temp: f32,
    pub max_temp: f32,
    pub avg_temp: f32,
    pub heater_ratio: f32,
}

#[derive(Debug, Clone)]
pub struct TelemetryRecord {
    pub node_id: u64,
    pub node_name: Option<String>,
    pub received_at: DateTime<Utc>,
    pub inside_temp: f32,
    pub heater_on: bool,
}

#[derive(Debug, Clone)]
pub struct NodeConfig {
    pub name: Option<String>,
    pub target_temp: Option<f32>,
    pub temp_tolerance: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct NodeDetails {
    pub name: Option<String>,
    pub target_temp: Option<f32>,
    pub temp_tolerance: Option<f32>,
    pub last_update: Option<DateTime<Utc>>,
    pub inside_temp: Option<f32>,
    pub heater_on: Option<bool>,
}

#[derive(Embed)]
#[folder = "./migrations/duckdb"]
struct Migrations;

impl Store {
    pub async fn open(file: impl AsRef<Path>) -> anyhow::Result<Self> {
        let file = file.as_ref().to_path_buf();
        let conn = task::spawn_blocking(move || -> anyhow::Result<Arc<Mutex<Connection>>> {
            let conn = Connection::open(file)?;

            conn.execute_batch(
                r#"
                    SET timezone='UTC';
                "#,
            )?;

            for migration in Migrations::iter() {
                tracing::info!("Executing migration {}", migration);
                let file = Migrations::get(&migration).expect("migration file");
                conn.execute_batch(std::str::from_utf8(&file.data)?)?;
            }

            Ok(Arc::new(Mutex::new(conn)))
        })
        .await??;

        Ok(Self { conn })
    }

    pub async fn save(&self, msg: TelemetryMessage) -> anyhow::Result<()> {
        let conn = self.conn.clone();
        task::spawn_blocking(move || {
            let conn = conn.lock().expect("poisoned mutex in metrics::Store::save");
            let mut stmt = conn.prepare_cached(
                r#"
                    INSERT INTO telemetry_raw (received_at, node_id, inside_temp, heater_on)
                    VALUES (now(), ?, ?, ?)
                "#,
            )?;
            stmt.execute(params![
                msg.sender_id as u64,
                msg.inside_temp as f32,
                msg.heater_enabled
            ])?;
            anyhow::Ok(())
        })
        .await?
    }

    pub async fn find_node_latest_telemetry(
        &self,
        node_id: u64,
    ) -> anyhow::Result<Option<TelemetryRecord>> {
        let conn = self.conn.clone();
        task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare_cached(
                r#"
                    SELECT
                        t.node_id,
                        t.received_at,
                        t.inside_temp,
                        t.heater_on,
                        nc.name
                    FROM telemetry_raw AS t
                    LEFT JOIN node_config  AS nc ON nc.node_id = t.node_id
                    WHERE
                        t.node_id = ?
                    ORDER BY t.received_at DESC
                    LIMIT 1;
                "#,
            )?;

            Ok(stmt
                .query_one(params![node_id], |row| {
                    Ok(TelemetryRecord {
                        node_id: row.get("node_id")?,
                        received_at: row.get("received_at")?,
                        inside_temp: row.get("inside_temp")?,
                        heater_on: row.get("heater_on")?,
                        node_name: row.get("name")?,
                    })
                })
                .optional()?)
        })
        .await?
    }

    pub async fn all_nodes_latest_record_in_range(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> anyhow::Result<Vec<TelemetryRecord>> {
        let conn = self.conn.clone();
        task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare_cached(
                r#"
                    SELECT DISTINCT ON (t.node_id)
                        t.node_id,
                        t.received_at,
                        t.inside_temp,
                        t.heater_on,
                        nc.name
                    FROM telemetry_raw AS t
                    LEFT JOIN node_config  AS nc ON nc.node_id = t.node_id
                    WHERE t.received_at BETWEEN ? AND ?
                    ORDER BY t.node_id, t.received_at DESC;
                "#,
            )?;

            let rows = stmt.query_map(params![from, to], |row| {
                Ok(TelemetryRecord {
                    node_id: row.get("node_id")?,
                    received_at: row.get("received_at")?,
                    inside_temp: row.get("inside_temp")?,
                    heater_on: row.get("heater_on")?,
                    node_name: row.get("name")?,
                })
            })?;

            Ok(rows.collect::<Result<Vec<TelemetryRecord>, _>>()?)
        })
        .await?
    }

    pub async fn range(
        &self,
        sender: u64,
        granularity: Granularity,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> anyhow::Result<Vec<DataPoint>> {
        let conn = self.conn.clone();
        task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            let sql = format!(
                r#"
                    SELECT
                        {ts_col} AS ts,
                        num_records,
                        inside_temp_min,
                        inside_temp_max,
                        inside_temp_avg,
                        heater_ratio
                    FROM {view}
                    WHERE
                        node_id = ?
                        AND {ts_col} BETWEEN ? AND ?
                    ORDER BY {ts_col}
                "#,
                view = granularity.view(),
                ts_col = granularity.ts_col(),
            );

            let mut stmt = conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(params![sender, from, to], |row| {
                Ok(DataPoint {
                    ts: row.get("ts")?,
                    num_records: row.get("num_records")?,
                    min_temp: row.get("inside_temp_min")?,
                    max_temp: row.get("inside_temp_max")?,
                    avg_temp: row.get("inside_temp_avg")?,
                    heater_ratio: row.get("heater_ratio")?,
                })
            })?;

            Ok(rows.collect::<Result<Vec<DataPoint>, _>>()?)
        })
        .await?
    }

    pub async fn find_node_details(&self, node_id: u64) -> anyhow::Result<Option<NodeDetails>> {
        let conn = self.conn.clone();
        task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare_cached(
                r#"
                    SELECT
                        t.received_at,
                        t.inside_temp,
                        t.heater_on,
                        nc.name,
                        nc.target_temp,
                        nc.temp_tolerance
                    FROM telemetry_raw AS t
                    LEFT JOIN node_config  AS nc ON nc.node_id = t.node_id
                    WHERE
                        t.node_id = ?
                    ORDER BY t.received_at DESC
                    LIMIT 1;
                "#,
            )?;
            Ok(stmt
                .query_one(params![node_id], |row| {
                    Ok(NodeDetails {
                        name: row.get("name")?,
                        target_temp: row.get("target_temp")?,
                        temp_tolerance: row.get("temp_tolerance")?,
                        last_update: row.get("received_at")?,
                        inside_temp: row.get("inside_temp")?,
                        heater_on: row.get("heater_on")?,
                    })
                })
                .optional()?)
        })
        .await?
    }

    pub async fn find_node_config(&self, node_id: u64) -> anyhow::Result<Option<NodeConfig>> {
        let conn = self.conn.clone();
        task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare_cached(
                r#"
                    SELECT
                        name,
                        target_temp,
                        temp_tolerance
                    FROM
                        node_config
                    WHERE
                        node_id = ?
                "#,
            )?;
            Ok(stmt
                .query_one(params![node_id], |row| {
                    Ok(NodeConfig {
                        name: row.get("name")?,
                        target_temp: row.get("target_temp")?,
                        temp_tolerance: row.get("temp_tolerance")?,
                    })
                })
                .optional()?)
        })
        .await?
    }

    pub async fn save_node_config(&self, node_id: u64, config: NodeConfig) -> anyhow::Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare_cached(
                r#"
                INSERT INTO node_config (node_id, name, target_temp, temp_tolerance)
                VALUES (?, ?, ?, ?)
                ON CONFLICT (node_id) DO UPDATE
                SET name        = COALESCE(excluded.name,        node_config.name),
                    target_temp = COALESCE(excluded.target_temp, node_config.target_temp),
                    temp_tolerance = COALESCE(excluded.temp_tolerance,
                                              node_config.temp_tolerance);
                "#,
            )?;
            stmt.execute(duckdb::params![
                node_id,
                config.name,
                config.target_temp,
                config.temp_tolerance,
            ])?;
            anyhow::Result::<()>::Ok(())
        })
        .await?
    }

    pub async fn set_node_name(&self, node_id: u64, new_name: &str) -> anyhow::Result<()> {
        let conn = self.conn.clone();
        let name = new_name.to_string();
        task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare_cached(
                r#"
                    INSERT INTO node_config (node_id, name)
                    VALUES (?, ?)
                "#,
            )?;
            stmt.execute(params![node_id, name])?;
            Ok(())
        })
        .await?
    }
}

#[async_trait::async_trait]
impl TelemetryProcessor for Store {
    async fn process(&self, msg: TelemetryMessage) {
        info!("Received telemetry {:?}", msg);
        match self.save(msg).await {
            Ok(_) => {
                debug!("Saved telemetry message");
            }
            Err(e) => {
                error!("Error saving telemetry: {}", e);
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;

    use chrono::TimeZone;
    use tempfile::TempDir;

    use super::*;

    async fn setup_test_db() -> anyhow::Result<(TempDir, Store)> {
        let tmp_dir = TempDir::new()?;
        let db_file = tmp_dir.path().join("test.db");
        let store = Store::open(&db_file).await?;
        Ok((tmp_dir, store))
    }

    async fn seed_metrics(store: &Store) -> anyhow::Result<()> {
        use std::fmt::Write;

        let base = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let mut sql = String::from(
            "INSERT INTO telemetry_raw (received_at, node_id, inside_temp, heater_on) VALUES ",
        );

        for min in 0..240u32 {
            let ts = base + Duration::seconds(min as i64 * 60);
            let temp = 20.0 + (min % 60) as f32;
            let heater = min % 5 == 0;

            write!(
                &mut sql,
                "('{}', 42, {}, {}),",
                ts.to_rfc3339(),
                temp,
                heater
            )
            .unwrap();
        }

        sql.pop();
        store.conn.lock().unwrap().execute(&sql, [])?;

        Ok(())
    }

    #[tokio::test]
    async fn save_and_retrieve_raw_data() -> anyhow::Result<()> {
        let (_dbf, store) = setup_test_db().await?;

        let msg = TelemetryMessage {
            sender_id: 77,
            inside_temp: 21.5,
            heater_enabled: true,
        };
        store.save(msg).await?;

        let conn = store.conn.lock().unwrap();
        let (id, temp, heater): (u64, f32, bool) = conn.query_row(
            r#"
                    SELECT node_id, inside_temp, heater_on
                    FROM telemetry_raw
                    WHERE node_id = ?
                "#,
            params![77],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;

        assert_eq!(id, 77);
        assert!((temp - 21.5).abs() < f32::EPSILON);
        assert!(heater);
        Ok(())
    }

    #[tokio::test]
    async fn minute_level_aggregation() -> anyhow::Result<()> {
        let (_dbf, store) = setup_test_db().await?;
        seed_metrics(&store).await?;

        let from = Utc.with_ymd_and_hms(2024, 1, 1, 2, 0, 0).unwrap();
        let to = Utc.with_ymd_and_hms(2024, 1, 1, 2, 14, 59).unwrap();

        let rows = store.range(42, Granularity::Minute, from, to).await?;
        assert_eq!(rows.len(), 15);

        let first = &rows[0];
        assert_eq!(first.ts, Utc.with_ymd_and_hms(2024, 1, 1, 2, 0, 0).unwrap());
        assert_eq!(first.num_records, 1);
        assert!((first.min_temp - 20.0).abs() < f32::EPSILON);
        assert!((first.max_temp - 20.0).abs() < f32::EPSILON);
        assert!((first.avg_temp - 20.0).abs() < f32::EPSILON);
        assert!((first.heater_ratio - 1.0).abs() < f32::EPSILON);

        assert_eq!(
            rows.last().unwrap().ts,
            Utc.with_ymd_and_hms(2024, 1, 1, 2, 14, 0).unwrap()
        );
        Ok(())
    }

    #[tokio::test]
    async fn hour_level_aggregation() -> anyhow::Result<()> {
        let (_dbf, store) = setup_test_db().await?;
        seed_metrics(&store).await?;

        let from = Utc.with_ymd_and_hms(2024, 1, 1, 1, 0, 0).unwrap();
        let to = Utc.with_ymd_and_hms(2024, 1, 1, 3, 0, 0).unwrap();

        let rows = store.range(42, Granularity::Hour, from, to).await?;
        assert_eq!(rows.len(), 3);

        // 2nd hour (index 1) has 60 cells => 12/60 heaters
        let second_hour = &rows[1];
        assert_eq!(
            second_hour.ts,
            Utc.with_ymd_and_hms(2024, 1, 1, 2, 0, 0).unwrap()
        );
        assert_eq!(second_hour.num_records, 60);
        assert_eq!(second_hour.heater_ratio, 12.0 / 60.0);
        // temps 20..79 ⇒ average = (20 + 79) / 2 = 49.5
        assert!((second_hour.avg_temp - 49.5).abs() < 0.01);
        Ok(())
    }

    #[tokio::test]
    async fn multiple_granularities_compared() -> anyhow::Result<()> {
        let (_dbf, store) = setup_test_db().await?;
        seed_metrics(&store).await?;

        let whole_day = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
            ..=Utc.with_ymd_and_hms(2024, 1, 1, 23, 59, 59).unwrap();

        for gran in &[Granularity::Minute, Granularity::Hour, Granularity::Day] {
            let start = whole_day.start().clone();
            let end = whole_day.end().clone();
            let rows = store.range(42, *gran, start, end).await?;

            // Minute: 240 rows, Hour: 4 rows, Day:1 row
            let expected = match gran {
                Granularity::Minute => 240,
                Granularity::Hour => 4,
                Granularity::Day => 1,
                _ => unreachable!(),
            };
            assert_eq!(rows.len(), expected, "granularity {:?} mismatched", gran);
        }

        Ok(())
    }

    #[tokio::test]
    async fn empty_range_returns_empty_vec() -> anyhow::Result<()> {
        let (_dbf, store) = setup_test_db().await?;
        let from = Utc::now();
        let to = from + Duration::seconds(24 * 60 * 60);

        let rows = store.range(999, Granularity::Day, from, to).await?;
        assert!(rows.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn latest_records_in_range() -> anyhow::Result<()> {
        let (_dbf, store) = setup_test_db().await?;
        let to = Utc::now();
        let from = to - Duration::minutes(60);

        let senders = vec![1u64, 2, 3];
        let metrics = vec![
            vec![
                (to - Duration::minutes(30), 30.0f32, true),
                (to - Duration::minutes(90), 90.0f32, true),
            ],
            vec![
                (to - Duration::minutes(15), 15.0f32, false),
                (to - Duration::minutes(30), 30.0f32, true),
            ],
            vec![
                (to - Duration::minutes(90), 90.0f32, false),
                (to - Duration::minutes(300), 300.0f32, false),
            ],
        ];

        let mut sql = String::from(
            "INSERT INTO telemetry_raw (received_at, node_id, inside_temp, heater_on) VALUES ",
        );

        for (sender, metrics) in senders.iter().zip(metrics.iter()) {
            for (ts, temp, heater) in metrics {
                write!(
                    &mut sql,
                    "('{}', {}, {}, {}),",
                    ts.to_rfc3339(),
                    sender,
                    temp,
                    heater
                )
                .unwrap();
            }
        }

        sql.pop();
        store.conn.lock().unwrap().execute(&sql, [])?;

        let actual = store.all_nodes_latest_record_in_range(from, to).await?;
        assert_eq!(actual.len(), 2);

        assert_eq!(actual[0].node_id, senders[0]);
        assert_eq!(
            actual[0].received_at.timestamp_micros(),
            metrics[0][0].0.timestamp_micros()
        );
        assert!((actual[0].inside_temp - metrics[0][0].1).abs() < f32::EPSILON);
        assert_eq!(actual[0].heater_on, metrics[0][0].2);

        assert_eq!(actual[1].node_id, senders[1]);
        assert_eq!(
            actual[1].received_at.timestamp_micros(),
            metrics[1][0].0.timestamp_micros()
        );
        assert!((actual[1].inside_temp - metrics[1][0].1).abs() < f32::EPSILON);
        assert_eq!(actual[1].heater_on, metrics[1][0].2);

        Ok(())
    }
}
