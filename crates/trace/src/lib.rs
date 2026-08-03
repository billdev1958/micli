//! Log de eventos append-only con hash encadenado. Fuente de verdad del
//! sistema: `hash(n) = H(hash(n-1) || evento(n))`.
//!
//! Fase 2: escritura solo en modo append, esquema de eventos versionado y
//! verificación de la cadena (`micli trace verify`). Nunca escribir lógica
//! que edite o borre entradas existentes.
