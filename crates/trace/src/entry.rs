//! Entradas del trace: el sobre con hash encadenado y los payloads de dominio.

use micli_core::{Event, RequirementId, State};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Hash previo de la primera entrada de la cadena (64 ceros).
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Hecho de dominio registrado en el trace. El trace es la fuente de verdad;
/// el contexto de cada hecho vive aquí, no en la máquina de estados.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Payload {
    RequirementCreado {
        id: RequirementId,
        titulo: String,
    },
    Transicion {
        requirement_id: RequirementId,
        evento: Event,
        desde: State,
        hacia: State,
    },
}

/// Una entrada del log append-only. `hash` cubre todos los demás campos
/// (incluido `prev_hash`), de modo que alterar cualquier entrada pasada
/// invalida toda la cadena posterior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceEntry {
    /// Versión del esquema serializado, para migraciones futuras.
    pub version: u32,
    /// Posición en la cadena, consecutiva desde 0.
    pub seq: u64,
    /// Milisegundos desde el epoch Unix.
    pub timestamp_ms: u64,
    /// Hash de la entrada anterior, o [`GENESIS_HASH`] si es la primera.
    pub prev_hash: String,
    pub payload: Payload,
    /// `hex(SHA-256(json(version, seq, timestamp_ms, prev_hash, payload)))`.
    pub hash: String,
}

/// Los campos cubiertos por el hash, en orden fijo. La serialización JSON de
/// un struct es determinista (sigue el orden de declaración), así que sirve
/// como forma canónica.
#[derive(Serialize)]
struct Preimage<'a> {
    version: u32,
    seq: u64,
    timestamp_ms: u64,
    prev_hash: &'a str,
    payload: &'a Payload,
}

impl TraceEntry {
    /// Versión actual del esquema serializado.
    pub const VERSION: u32 = 1;

    /// Construye la entrada calculando su hash a partir del resto de campos.
    pub fn nueva(
        seq: u64,
        timestamp_ms: u64,
        prev_hash: String,
        payload: Payload,
    ) -> Result<Self, serde_json::Error> {
        let mut entrada = Self {
            version: Self::VERSION,
            seq,
            timestamp_ms,
            prev_hash,
            payload,
            hash: String::new(),
        };
        entrada.hash = entrada.calcular_hash()?;
        Ok(entrada)
    }

    /// Recalcula el hash desde los campos actuales (sin usar `self.hash`).
    pub fn calcular_hash(&self) -> Result<String, serde_json::Error> {
        let preimagen = Preimage {
            version: self.version,
            seq: self.seq,
            timestamp_ms: self.timestamp_ms,
            prev_hash: &self.prev_hash,
            payload: &self.payload,
        };
        let bytes = serde_json::to_vec(&preimagen)?;
        let digest = Sha256::digest(&bytes);
        let mut hex = String::with_capacity(64);
        for byte in digest {
            hex.push_str(&format!("{byte:02x}"));
        }
        Ok(hex)
    }
}
