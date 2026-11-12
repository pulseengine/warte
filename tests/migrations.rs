//! A released migration is never edited: sqlx checksums the whole file and
//! refuses to start on a database that applied a different checksum. v1.6.23
//! edited a comment in 005 and did not start on a deployed database (REQ-STORE-003).

use sha2::{Digest, Sha384};
use warte::db::{Database, RELEASED_MIGRATIONS};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

// rivet: verifies REQ-STORE-003
#[test]
fn released_migrations_are_never_edited() {
    // fail closed: an empty ledger checks nothing
    assert!(RELEASED_MIGRATIONS.len() >= 7, "the ledger lost entries");
    let last = RELEASED_MIGRATIONS.iter().map(|m| m.0).max().unwrap();
    for (version, file, want) in RELEASED_MIGRATIONS {
        let path = format!("{}/migrations/{file}", env!("CARGO_MANIFEST_DIR"));
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{file}: {e}"));
        assert_eq!(
            hex::encode(Sha384::digest(&bytes)),
            *want,
            "{file} was edited after release; restore it and put the change in a new migration"
        );
        let m = MIGRATOR
            .iter()
            .find(|m| m.version == *version)
            .unwrap_or_else(|| panic!("migration {version} is gone"));
        assert_eq!(hex::encode(&m.checksum), *want, "sqlx's checksum of {file}");
    }
    // a migration not in the ledger is new, so it comes after every released one
    for m in MIGRATOR.iter() {
        if !RELEASED_MIGRATIONS.iter().any(|r| r.0 == m.version) {
            assert!(
                m.version > last,
                "migration {} is new but not last",
                m.version
            );
        }
    }
}

// rivet: verifies REQ-STORE-003
#[tokio::test]
async fn a_database_from_an_earlier_release_still_opens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = Database::connect(&path).await.unwrap();
    // record the checksums the released binaries wrote, as a deployed database has them
    for (version, _, sum) in RELEASED_MIGRATIONS {
        sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?")
            .bind(hex::decode(sum).unwrap())
            .bind(version)
            .execute(db.pool())
            .await
            .unwrap();
    }
    db.pool().close().await;
    Database::connect(&path)
        .await
        .expect("this build must start on a database an earlier release migrated");

    // negative control: a database that recorded another checksum is refused
    let db = Database::connect(&path).await.unwrap();
    sqlx::query("UPDATE _sqlx_migrations SET checksum = x'00' WHERE version = 5")
        .execute(db.pool())
        .await
        .unwrap();
    db.pool().close().await;
    let err = Database::connect(&path).await.err().expect("refused");
    assert!(format!("{err:#}").contains("modified"), "{err:#}");
}
