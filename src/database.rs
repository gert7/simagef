use std::{
    fs::Metadata, time::UNIX_EPOCH
};

use rusqlite::{Connection, params};

#[derive(Debug)]
struct SignatureRow {
    id: i64,
    path: String,
    modified: i64,
    pub signature: Vec<u8>,
}

#[derive(Debug)]
pub struct Signature {
    id: i64,
    path: String,
    modified: u64,
    pub signature: Vec<i8>,
}

pub fn init(db_conn: &Connection) -> rusqlite::Result<usize> {
    db_conn.execute(
        "CREATE TABLE IF NOT EXISTS signatures (
                            path      TEXT NOT NULL PRIMARY KEY,
                            modified  INTEGER NOT NULL,
                            signature BLOB)",
        (),
    )
}

pub fn fetch(conn: &Connection, filename: &str, stat: &Metadata) -> anyhow::Result<Option<Signature>> {
    let mut stmt =
        conn.prepare("SELECT path, modified, signature FROM signatures WHERE path = (?1)")?;
    let mut signatures = stmt.query_map([filename], |row| {
        Ok(SignatureRow {
            id: 0,
            path: row.get(0)?,
            modified: row.get(1)?,
            signature: row.get(2)?,
        })
    })?;

    match signatures.next() {
        Some(sig) => {
            let sig = sig?;
            let modified = bytemuck::cast::<i64, u64>(sig.modified);
            if modified < stat.modified()?.duration_since(UNIX_EPOCH)?.as_secs() {
                Ok(None)
            } else {
                Ok(Some(Signature {
                    id: sig.id,
                    path: sig.path,
                    modified,
                    signature: bytemuck::cast_slice(&sig.signature).to_vec(),
                }))
            }
        }
        None => Ok(None),
    }
}

pub struct InsertionMessage {
    pub filename_s: String,
    pub stat: Metadata,
    pub signature: Vec<i8>,
}

pub fn insert_batch (
    conn: &mut Connection,
    messages: &Vec<InsertionMessage>,
) -> anyhow::Result<()> {

    let tx = conn.transaction()?;

    for msg in messages {
        let filename = &msg.filename_s;
        let modified = msg.stat.modified()?;
        let since: u64 = modified.duration_since(UNIX_EPOCH)?.as_secs().try_into()?;
        let since = bytemuck::cast::<u64, i64>(since);
        let signature = bytemuck::cast_slice::<i8, u8>(&msg.signature).to_vec();

        tx.execute(
            "INSERT OR REPLACE INTO signatures
                            (path, modified, signature)
                            VALUES
                            (?1, ?2, ?3)",
            params![filename, since, signature],
        )?;
    }

    tx.commit()?;
    Ok(())
}

pub fn insert(
    conn: &Connection,
    filename: &str,
    stat: &Metadata,
    signature: &[i8],
) -> anyhow::Result<usize> {
    let modified = stat.modified()?;
    let since: u64 = modified.duration_since(UNIX_EPOCH)?.as_secs().try_into()?;
    let since = bytemuck::cast::<u64, i64>(since);
    let signature = bytemuck::cast_slice::<i8, u8>(signature).to_vec();

    Ok(conn.execute(
        "INSERT OR REPLACE INTO signatures
                        (path, modified, signature)
                        VALUES
                        (?1, ?2, ?3)",
        params![filename, since, signature],
    )?)
}
