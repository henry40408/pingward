seed_1800000000000.sql is the output of the former Rust e2e/src/seed.rs
seed_sql(1_800_000_000_000), captured once in a container before the Rust code
was removed. The Go port must reproduce it byte for byte, which pins the
Mulberry32 draw order and the float arithmetic.
