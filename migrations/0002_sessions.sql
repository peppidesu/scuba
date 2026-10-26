CREATE TABLE sessions (
    id      TEXT PRIMARY KEY,
    data    TEXT NOT NULL,      -- JSON-encoded tower-sessions Record
    expiry  INTEGER NOT NULL    -- unix seconds
) STRICT;
CREATE INDEX sessions_expiry ON sessions (expiry);
