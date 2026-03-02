PRAGMA journal_mode=WAL;

CREATE TABLE IF NOT EXISTS healthcheck (
  id INTEGER PRIMARY KEY,
  label TEXT NOT NULL
);

DELETE FROM healthcheck;
INSERT INTO healthcheck (id, label) VALUES
  (1, 'inspector-ready'),
  (2, 'seed-loaded');

CREATE TABLE IF NOT EXISTS sample_items (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  qty INTEGER NOT NULL
);

DELETE FROM sample_items;
INSERT INTO sample_items (id, name, qty) VALUES
  (1, 'alpha', 3),
  (2, 'beta', 7),
  (3, 'gamma', 11);
