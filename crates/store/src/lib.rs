//! Proyección SQLite derivada del trace. Nunca es fuente de verdad: se
//! construye leyendo el trace y se puede regenerar desde cero
//! (`micli db rebuild`, vía [`Store::reset`] + reproyección).
//!
//! Este crate solo depende de `core`: recibe hechos ya tipados (con el hash
//! del evento del trace como ancla) y los proyecta a SQLite. El wiring
//! trace→store lo hace el binario `micli`.

use micli_core::{Event, State};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

/// Versión del esquema SQLite (se guarda en `PRAGMA user_version`).
const SCHEMA_VERSION: i32 = 1;

/// Errores de la proyección.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("error de SQLite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("dato corrupto en la proyección (¿DB desalineada del trace?): {0}")]
    Json(#[from] serde_json::Error),
    #[error(
        "esquema de la DB no soportado: {encontrada} (esta versión de micli soporta {soportada}); regenera con `micli db rebuild`"
    )]
    VersionNoSoportada { encontrada: i32, soportada: i32 },
    #[error("valor numérico fuera de rango para SQLite: {valor}")]
    FueraDeRango { valor: u64 },
}

/// Fila proyectada de `requirements`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementRow {
    pub id: String,
    pub titulo: String,
    pub estado: State,
    /// Hash del evento del trace que creó el requerimiento.
    pub hash_evento: String,
}

/// Fila proyectada de `transitions`, anclada al hash del evento del trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionRow {
    pub seq: u64,
    pub requirement_id: String,
    pub evento: Event,
    pub desde: State,
    pub hacia: State,
    pub hash_evento: String,
}

/// Intento de implementación registrado (lo alimentará el agente en Fase 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptRow {
    pub requirement_id: String,
    pub numero: u32,
    pub resultado: String,
    pub tokens: u64,
    pub duracion_ms: u64,
    pub hash_evento: String,
}

/// Resultado de un gate sobre un requerimiento.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateResultRow {
    pub requirement_id: String,
    pub gate: String,
    pub paso: bool,
    pub hash_evento: String,
}

/// Conexión a la proyección SQLite de un proyecto.
#[derive(Debug)]
pub struct Store {
    conn: Connection,
}

fn a_i64(valor: u64) -> Result<i64, StoreError> {
    i64::try_from(valor).map_err(|_| StoreError::FueraDeRango { valor })
}

impl Store {
    /// Abre (o crea) la DB en `path`: activa WAL y claves foráneas, y crea
    /// el esquema si la DB es nueva.
    pub fn abrir(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::inicializar(conn)
    }

    /// DB en memoria, para tests.
    pub fn en_memoria() -> Result<Self, StoreError> {
        Self::inicializar(Connection::open_in_memory()?)
    }

    fn inicializar(conn: Connection) -> Result<Self, StoreError> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let encontrada: i32 = conn.query_row("PRAGMA user_version", [], |f| f.get(0))?;
        match encontrada {
            0 => {
                conn.execute_batch(
                    "BEGIN;
                     CREATE TABLE requirements (
                       id          TEXT PRIMARY KEY,
                       titulo      TEXT NOT NULL,
                       estado      TEXT NOT NULL,
                       hash_evento TEXT NOT NULL UNIQUE
                     );
                     CREATE TABLE transitions (
                       seq            INTEGER PRIMARY KEY,
                       requirement_id TEXT NOT NULL REFERENCES requirements(id),
                       evento         TEXT NOT NULL,
                       desde          TEXT NOT NULL,
                       hacia          TEXT NOT NULL,
                       hash_evento    TEXT NOT NULL UNIQUE
                     );
                     CREATE TABLE attempts (
                       id             INTEGER PRIMARY KEY AUTOINCREMENT,
                       requirement_id TEXT NOT NULL REFERENCES requirements(id),
                       numero         INTEGER NOT NULL,
                       resultado      TEXT NOT NULL,
                       tokens         INTEGER NOT NULL,
                       duracion_ms    INTEGER NOT NULL,
                       hash_evento    TEXT NOT NULL UNIQUE
                     );
                     CREATE TABLE gate_results (
                       id             INTEGER PRIMARY KEY AUTOINCREMENT,
                       requirement_id TEXT NOT NULL REFERENCES requirements(id),
                       gate           TEXT NOT NULL,
                       paso           INTEGER NOT NULL,
                       hash_evento    TEXT NOT NULL
                     );
                     COMMIT;",
                )?;
                conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            }
            v if v == SCHEMA_VERSION => {}
            v => {
                return Err(StoreError::VersionNoSoportada {
                    encontrada: v,
                    soportada: SCHEMA_VERSION,
                });
            }
        }
        Ok(Self { conn })
    }

    /// Borra todas las filas proyectadas (paso previo de `micli db rebuild`).
    /// El esquema se conserva; el trace, intacto, permite reproyectar todo.
    pub fn reset(&mut self) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(
            "DELETE FROM gate_results;
             DELETE FROM attempts;
             DELETE FROM transitions;
             DELETE FROM requirements;",
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Proyecta la creación de un requerimiento (nace en `Draft`).
    pub fn proyectar_requirement_creado(
        &mut self,
        id: &str,
        titulo: &str,
        hash_evento: &str,
    ) -> Result<(), StoreError> {
        let estado = serde_json::to_string(&State::Draft)?;
        self.conn.execute(
            "INSERT INTO requirements (id, titulo, estado, hash_evento) VALUES (?1, ?2, ?3, ?4)",
            params![id, titulo, estado, hash_evento],
        )?;
        Ok(())
    }

    /// Proyecta una transición: inserta la fila anclada al hash del evento y
    /// actualiza el estado actual del requerimiento, en una transacción.
    pub fn proyectar_transicion(&mut self, fila: &TransitionRow) -> Result<(), StoreError> {
        let seq = a_i64(fila.seq)?;
        let evento = serde_json::to_string(&fila.evento)?;
        let desde = serde_json::to_string(&fila.desde)?;
        let hacia = serde_json::to_string(&fila.hacia)?;
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO transitions (seq, requirement_id, evento, desde, hacia, hash_evento)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                seq,
                fila.requirement_id,
                evento,
                desde,
                hacia,
                fila.hash_evento
            ],
        )?;
        let actualizadas = tx.execute(
            "UPDATE requirements SET estado = ?1 WHERE id = ?2",
            params![hacia, fila.requirement_id],
        )?;
        if actualizadas == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows.into());
        }
        tx.commit()?;
        Ok(())
    }

    /// Proyecta un intento de implementación.
    pub fn proyectar_attempt(&mut self, fila: &AttemptRow) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO attempts (requirement_id, numero, resultado, tokens, duracion_ms, hash_evento)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                fila.requirement_id,
                fila.numero,
                fila.resultado,
                a_i64(fila.tokens)?,
                a_i64(fila.duracion_ms)?,
                fila.hash_evento
            ],
        )?;
        Ok(())
    }

    /// Proyecta el resultado de un gate.
    pub fn proyectar_gate_result(&mut self, fila: &GateResultRow) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO gate_results (requirement_id, gate, paso, hash_evento)
             VALUES (?1, ?2, ?3, ?4)",
            params![fila.requirement_id, fila.gate, fila.paso, fila.hash_evento],
        )?;
        Ok(())
    }

    /// Busca un requerimiento por id.
    pub fn requirement(&self, id: &str) -> Result<Option<RequirementRow>, StoreError> {
        let fila = self
            .conn
            .query_row(
                "SELECT id, titulo, estado, hash_evento FROM requirements WHERE id = ?1",
                params![id],
                |f| {
                    Ok((
                        f.get::<_, String>(0)?,
                        f.get::<_, String>(1)?,
                        f.get::<_, String>(2)?,
                        f.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()?;
        match fila {
            None => Ok(None),
            Some((id, titulo, estado, hash_evento)) => Ok(Some(RequirementRow {
                id,
                titulo,
                estado: serde_json::from_str(&estado)?,
                hash_evento,
            })),
        }
    }

    /// Historial de transiciones de un requerimiento, en orden de trace.
    pub fn transiciones(&self, requirement_id: &str) -> Result<Vec<TransitionRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, requirement_id, evento, desde, hacia, hash_evento
             FROM transitions WHERE requirement_id = ?1 ORDER BY seq",
        )?;
        let filas = stmt.query_map(params![requirement_id], |f| {
            Ok((
                f.get::<_, i64>(0)?,
                f.get::<_, String>(1)?,
                f.get::<_, String>(2)?,
                f.get::<_, String>(3)?,
                f.get::<_, String>(4)?,
                f.get::<_, String>(5)?,
            ))
        })?;
        let mut resultado = Vec::new();
        for fila in filas {
            let (seq, requirement_id, evento, desde, hacia, hash_evento) = fila?;
            resultado.push(TransitionRow {
                seq: u64::try_from(seq).unwrap_or(0),
                requirement_id,
                evento: serde_json::from_str(&evento)?,
                desde: serde_json::from_str(&desde)?,
                hacia: serde_json::from_str(&hacia)?,
                hash_evento,
            });
        }
        Ok(resultado)
    }

    /// Todos los requerimientos proyectados, ordenados por id.
    pub fn requirements(&self) -> Result<Vec<RequirementRow>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, titulo, estado, hash_evento FROM requirements ORDER BY id")?;
        let filas = stmt.query_map([], |f| {
            Ok((
                f.get::<_, String>(0)?,
                f.get::<_, String>(1)?,
                f.get::<_, String>(2)?,
                f.get::<_, String>(3)?,
            ))
        })?;
        let mut resultado = Vec::new();
        for fila in filas {
            let (id, titulo, estado, hash_evento) = fila?;
            resultado.push(RequirementRow {
                id,
                titulo,
                estado: serde_json::from_str(&estado)?,
                hash_evento,
            });
        }
        Ok(resultado)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transicion(seq: u64, id: &str, evento: Event, desde: State, hacia: State) -> TransitionRow {
        TransitionRow {
            seq,
            requirement_id: id.to_owned(),
            evento,
            desde,
            hacia,
            hash_evento: format!("hash-{seq}"),
        }
    }

    fn store_con_requirement() -> Store {
        let mut store = Store::en_memoria().unwrap();
        store
            .proyectar_requirement_creado("REQ-001", "Alta de usuarios", "hash-0")
            .unwrap();
        store
    }

    #[test]
    fn proyecta_creacion_y_estado_inicial_draft() {
        let store = store_con_requirement();
        let fila = store.requirement("REQ-001").unwrap().unwrap();
        assert_eq!(fila.titulo, "Alta de usuarios");
        assert_eq!(fila.estado, State::Draft);
        assert_eq!(fila.hash_evento, "hash-0");
        assert!(store.requirement("REQ-999").unwrap().is_none());
    }

    #[test]
    fn proyectar_transicion_actualiza_el_estado_actual() {
        let mut store = store_con_requirement();
        store
            .proyectar_transicion(&transicion(
                1,
                "REQ-001",
                Event::AprobarSpec,
                State::Draft,
                State::SpecAprobada,
            ))
            .unwrap();
        store
            .proyectar_transicion(&transicion(
                2,
                "REQ-001",
                Event::Bloquear,
                State::SpecAprobada,
                State::Bloqueado {
                    desde: micli_core::ActiveState::SpecAprobada,
                },
            ))
            .unwrap();

        let fila = store.requirement("REQ-001").unwrap().unwrap();
        assert_eq!(
            fila.estado,
            State::Bloqueado {
                desde: micli_core::ActiveState::SpecAprobada
            }
        );
        let historial = store.transiciones("REQ-001").unwrap();
        assert_eq!(historial.len(), 2);
        assert_eq!(historial[0].hacia, State::SpecAprobada);
        assert_eq!(historial[1].hash_evento, "hash-2");
    }

    #[test]
    fn rechaza_transicion_de_requirement_inexistente() {
        let mut store = Store::en_memoria().unwrap();
        let error = store.proyectar_transicion(&transicion(
            1,
            "REQ-404",
            Event::AprobarSpec,
            State::Draft,
            State::SpecAprobada,
        ));
        assert!(error.is_err());
    }

    #[test]
    fn rechaza_hash_de_evento_duplicado() {
        let mut store = store_con_requirement();
        let fila = transicion(
            1,
            "REQ-001",
            Event::AprobarSpec,
            State::Draft,
            State::SpecAprobada,
        );
        store.proyectar_transicion(&fila).unwrap();
        assert!(store.proyectar_transicion(&fila).is_err());
    }

    #[test]
    fn reset_y_reproyeccion_reconstruyen_lo_mismo() {
        let mut store = store_con_requirement();
        let fila = transicion(
            1,
            "REQ-001",
            Event::AprobarSpec,
            State::Draft,
            State::SpecAprobada,
        );
        store.proyectar_transicion(&fila).unwrap();
        let antes = store.requirements().unwrap();

        store.reset().unwrap();
        assert!(store.requirements().unwrap().is_empty());

        store
            .proyectar_requirement_creado("REQ-001", "Alta de usuarios", "hash-0")
            .unwrap();
        store.proyectar_transicion(&fila).unwrap();
        assert_eq!(store.requirements().unwrap(), antes);
    }

    #[test]
    fn attempts_y_gate_results_se_proyectan() {
        let mut store = store_con_requirement();
        store
            .proyectar_attempt(&AttemptRow {
                requirement_id: "REQ-001".to_owned(),
                numero: 1,
                resultado: "gate falló: tests".to_owned(),
                tokens: 12_345,
                duracion_ms: 60_000,
                hash_evento: "hash-9".to_owned(),
            })
            .unwrap();
        store
            .proyectar_gate_result(&GateResultRow {
                requirement_id: "REQ-001".to_owned(),
                gate: "tests".to_owned(),
                paso: false,
                hash_evento: "hash-9".to_owned(),
            })
            .unwrap();
        // Otra corrida puede repetir el numero de intento (se reinicia por
        // corrida); lo que nunca se repite es el hash del evento.
        store
            .proyectar_attempt(&AttemptRow {
                requirement_id: "REQ-001".to_owned(),
                numero: 1,
                resultado: "otra corrida".to_owned(),
                tokens: 1,
                duracion_ms: 1,
                hash_evento: "hash-10".to_owned(),
            })
            .unwrap();
        assert!(
            store
                .proyectar_attempt(&AttemptRow {
                    requirement_id: "REQ-001".to_owned(),
                    numero: 2,
                    resultado: "hash duplicado".to_owned(),
                    tokens: 1,
                    duracion_ms: 1,
                    hash_evento: "hash-10".to_owned(),
                })
                .is_err()
        );
    }

    #[test]
    fn abrir_en_disco_activa_wal_y_persiste() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("micli.db");
        {
            let mut store = Store::abrir(&path).unwrap();
            let modo: String = store
                .conn
                .query_row("PRAGMA journal_mode", [], |f| f.get(0))
                .unwrap();
            assert_eq!(modo.to_lowercase(), "wal");
            store
                .proyectar_requirement_creado("REQ-001", "Alta", "hash-0")
                .unwrap();
        }
        let store = Store::abrir(&path).unwrap();
        assert!(store.requirement("REQ-001").unwrap().is_some());
    }

    #[test]
    fn rechaza_version_de_esquema_futura() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("micli.db");
        drop(Store::abrir(&path).unwrap());

        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
        drop(conn);

        let error = Store::abrir(&path).unwrap_err();
        assert!(matches!(
            error,
            StoreError::VersionNoSoportada { encontrada: 99, .. }
        ));
    }
}
