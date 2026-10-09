use rusqlite::Connection;

pub fn init(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS entries (
            id        INTEGER PRIMARY KEY,
            lemma     TEXT NOT NULL,
            pos       TEXT NOT NULL,
            senses    TEXT NOT NULL,   -- JSON: [{\"gloss\": \"...\", \"examples\": [\"...\"]}]
            hanja     TEXT,
            etymology TEXT
        );
        CREATE INDEX IF NOT EXISTS entries_lemma ON entries(lemma);

        -- inflected/alternative form -> lemma (from Wiktionary form-of senses)
        CREATE TABLE IF NOT EXISTS forms (
            form  TEXT NOT NULL,
            lemma TEXT NOT NULL,
            PRIMARY KEY (form, lemma)
        );

        CREATE TABLE IF NOT EXISTS levels (
            lemma TEXT PRIMARY KEY,
            level TEXT NOT NULL
        );
        ",
    )
}
