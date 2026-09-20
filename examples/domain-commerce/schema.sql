CREATE TABLE customers (
  id TEXT PRIMARY KEY,
  email TEXT NOT NULL,
  display_name TEXT NOT NULL
);

CREATE TABLE orders (
  id TEXT PRIMARY KEY,
  customer_id TEXT NOT NULL REFERENCES customers(id),
  total_cents BIGINT NOT NULL,
  status TEXT NOT NULL
);
