//! SQLite database layer for warte state.

use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::path::Path;

/// Every released migration and the SHA-384 of its file, as sqlx records it
/// in `_sqlx_migrations`. A released migration is never edited, not even a
/// comment: sqlx refuses to start on a database that applied a different
/// checksum ("migration N was previously applied but has been modified").
/// A change goes into a new migration, appended here once it is released.
pub const RELEASED_MIGRATIONS: &[(i64, &str, &str)] = &[
    (
        1,
        "001_initial.sql",
        "d821568af0c0881642fb0a532c277816b61812d63d306ca6de676a70a5a0961db11aaa0478d9af29fd112068e11144f9",
    ),
    (
        2,
        "002_orset_tombstones.sql",
        "f8332efea301bba1d66bfe47bb61bb7992ffa2fd45eacf827df25dae989b5ad50abc15b545aea85aef5b28bb93d32d00",
    ),
    (
        3,
        "003_wait_history.sql",
        "f8ce967ece36d56c6386ae054b4708aebd87449d1f1012b2d0928aaeabc36b5b0690829ad11a3d36f0f947a83f06a4f4",
    ),
    (
        4,
        "004_ci_jobs.sql",
        "e97c735cc0f0b6407eb6bb4e1bcc3fccd66efdb6f0f9f84739f3d3847fb4fd3aace057544a6d7c7331c5af470efad776",
    ),
    (
        5,
        "005_drop_ci_telemetry.sql",
        "3c2979dce8d383f68f8faed738857445d8d15a75d35c23a1884e3a6d797a5e92458b4b4f153691b658800909eab13ae8",
    ),
    (
        6,
        "006_probe_tokens.sql",
        "6090ff80a43d9467cef882b4e182cb03be6536bbb504e5e243192cc21a4018406e04b9ef9ff53fad76fd0311873c41af",
    ),
    (
        7,
        "007_probe_revocations.sql",
        "748cc63d871ba606d76bff71816d8627dea47662a53b997af9c80c736c9d26cea3b3f2603e7c24683a1681fc3d444e72",
    ),
];

/// Database connection pool.
#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    /// Connect to the database, creating it if it doesn't exist.
    pub async fn connect(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let url = format!("sqlite://{}?mode=rwc", path.display());
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await?;

        // Run migrations
        sqlx::migrate!("./migrations").run(&pool).await?;

        Ok(Self { pool })
    }

    /// Get the connection pool.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}
