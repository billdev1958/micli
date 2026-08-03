//! Proyección SQLite derivada del trace. Nunca es fuente de verdad: siempre
//! reconstruible desde el log encadenado (`micli db rebuild`).
//!
//! Fase 2: esquema `requirements`, `transitions`, `attempts`, `gate_results`
//! con `rusqlite` en modo WAL y escrituras transaccionales.
