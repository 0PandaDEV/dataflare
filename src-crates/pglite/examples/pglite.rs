use pglite::Connection;
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    let path = PathBuf::from(file!())
        .parent()
        .unwrap()
        .join(".pglite-data");
    let conn = Connection::connect(path.to_string_lossy().as_ref())
        .await
        .unwrap();

    conn.execute(
        "
        CREATE TABLE IF NOT EXISTS messages (
            id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            body text NOT NULL
        );
        INSERT INTO messages (body) VALUES ('hello from PGlite');
        ",
    )
    .unwrap();

    let query = conn
        .query("SELECT id, body FROM messages ORDER BY id")
        .unwrap();
    dbg!(query);
}
