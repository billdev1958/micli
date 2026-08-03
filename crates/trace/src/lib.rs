//! Log de eventos append-only con hash encadenado: la fuente de verdad de
//! cada proyecto registrado en `micli`.
//!
//! Cada entrada es una línea JSON (`JSONL`) cuyo hash cubre la entrada
//! completa, incluido el hash de la anterior:
//! `hash(n) = SHA-256(entrada(n) con prev_hash = hash(n-1))`. Alterar una
//! entrada pasada invalida toda la cadena posterior; [`Trace::verificar`] lo
//! comprueba.
//!
//! El archivo se abre exclusivamente en modo append (regla dura #5). La
//! única alteración que la cadena no puede detectar por sí sola es truncar
//! la cola del archivo; ese anclaje externo llegará con la proyección
//! (`store`) y el índice global.

mod entry;

pub use entry::{GENESIS_HASH, Payload, TraceEntry};

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Errores del trace.
#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    #[error("error de E/S en el trace: {0}")]
    Io(#[from] std::io::Error),
    #[error("entrada del trace ilegible: {0}")]
    Json(#[from] serde_json::Error),
    #[error("la entrada {seq} fue alterada: su hash no coincide con su contenido")]
    HashCorrupto { seq: u64 },
    #[error("la entrada {seq} no enlaza con la anterior: prev_hash no coincide")]
    EnlaceRoto { seq: u64 },
    #[error("secuencia rota: se esperaba la entrada {esperada} y se encontró {encontrada}")]
    SeqInvalida { esperada: u64, encontrada: u64 },
}

/// Handle de escritura/lectura sobre el trace de un proyecto.
///
/// Mantiene en memoria la punta de la cadena (`proxima_seq`, `ultimo_hash`)
/// para que cada [`Trace::append`] sea O(1). Asume un único escritor por
/// proyecto.
#[derive(Debug)]
pub struct Trace {
    path: PathBuf,
    proxima_seq: u64,
    ultimo_hash: String,
}

impl Trace {
    /// Abre (o crea) el trace en `path`. Si el archivo existe, retoma la
    /// cadena desde su última entrada.
    pub fn abrir(path: impl Into<PathBuf>) -> Result<Self, TraceError> {
        let path = path.into();
        if let Some(padre) = path.parent()
            && !padre.as_os_str().is_empty()
        {
            fs::create_dir_all(padre)?;
        }
        let (proxima_seq, ultimo_hash) = match fs::read_to_string(&path) {
            Ok(contenido) => match contenido.lines().rev().find(|l| !l.trim().is_empty()) {
                Some(linea) => {
                    let ultima: TraceEntry = serde_json::from_str(linea)?;
                    (ultima.seq + 1, ultima.hash)
                }
                None => (0, GENESIS_HASH.to_owned()),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (0, GENESIS_HASH.to_owned()),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            path,
            proxima_seq,
            ultimo_hash,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Agrega un evento al final de la cadena y lo persiste. El archivo se
    /// abre solo en modo append: nunca se reescriben entradas anteriores.
    pub fn append(&mut self, payload: Payload) -> Result<TraceEntry, TraceError> {
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0);
        let entrada = TraceEntry::nueva(
            self.proxima_seq,
            timestamp_ms,
            self.ultimo_hash.clone(),
            payload,
        )?;

        let mut linea = serde_json::to_string(&entrada)?;
        linea.push('\n');
        let mut archivo = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&self.path)?;
        archivo.write_all(linea.as_bytes())?;
        archivo.flush()?;

        self.proxima_seq = entrada.seq + 1;
        self.ultimo_hash = entrada.hash.clone();
        Ok(entrada)
    }

    /// Lee y deserializa todas las entradas, sin verificar la cadena.
    pub fn entradas(&self) -> Result<Vec<TraceEntry>, TraceError> {
        let contenido = match fs::read_to_string(&self.path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut entradas = Vec::new();
        for linea in contenido.lines().filter(|l| !l.trim().is_empty()) {
            entradas.push(serde_json::from_str(linea)?);
        }
        Ok(entradas)
    }

    /// Verifica la cadena completa: secuencia consecutiva desde 0, enlace de
    /// `prev_hash` y hash de cada entrada. Devuelve cuántas entradas verificó.
    pub fn verificar(&self) -> Result<u64, TraceError> {
        let mut esperada: u64 = 0;
        let mut prev_hash = GENESIS_HASH.to_owned();
        for entrada in self.entradas()? {
            if entrada.seq != esperada {
                return Err(TraceError::SeqInvalida {
                    esperada,
                    encontrada: entrada.seq,
                });
            }
            if entrada.prev_hash != prev_hash {
                return Err(TraceError::EnlaceRoto { seq: entrada.seq });
            }
            if entrada.calcular_hash()? != entrada.hash {
                return Err(TraceError::HashCorrupto { seq: entrada.seq });
            }
            prev_hash = entrada.hash;
            esperada += 1;
        }
        Ok(esperada)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use micli_core::{Event, RequirementId, State};

    fn payload_creacion(n: u32) -> Payload {
        Payload::RequirementCreado {
            id: RequirementId::new(format!("REQ-{n:03}")),
            titulo: format!("Requerimiento {n}"),
        }
    }

    fn trace_temporal() -> (tempfile::TempDir, Trace) {
        let dir = tempfile::tempdir().unwrap();
        let trace = Trace::abrir(dir.path().join("trace.jsonl")).unwrap();
        (dir, trace)
    }

    #[test]
    fn append_encadena_y_verificar_pasa() {
        let (_dir, mut trace) = trace_temporal();
        for n in 0..5 {
            trace.append(payload_creacion(n)).unwrap();
        }
        let entradas = trace.entradas().unwrap();
        assert_eq!(entradas.len(), 5);
        assert_eq!(entradas[0].prev_hash, GENESIS_HASH);
        for par in entradas.windows(2) {
            assert_eq!(par[1].prev_hash, par[0].hash);
        }
        assert_eq!(trace.verificar().unwrap(), 5);
    }

    #[test]
    fn registra_transiciones_con_tipos_de_core() {
        let (_dir, mut trace) = trace_temporal();
        trace.append(payload_creacion(1)).unwrap();
        trace
            .append(Payload::Transicion {
                requirement_id: RequirementId::new("REQ-001"),
                evento: Event::AprobarSpec,
                desde: State::Draft,
                hacia: State::SpecAprobada,
            })
            .unwrap();
        assert_eq!(trace.verificar().unwrap(), 2);
    }

    #[test]
    fn registra_intentos_y_resultados_de_gates() {
        let (_dir, mut trace) = trace_temporal();
        trace
            .append(Payload::Intento {
                requirement_id: RequirementId::new("REQ-001"),
                numero: 1,
                estrategia: "Directo".to_owned(),
                tokens: 1500,
                duracion_ms: 60_000,
                gates_pasaron: false,
                resumen: "falló gate tests".to_owned(),
                hash_prompt_sistema: "abc123".to_owned(),
            })
            .unwrap();
        trace
            .append(Payload::GateResultado {
                requirement_id: RequirementId::new("REQ-001"),
                intento: 1,
                gate: "tests".to_owned(),
                paso: false,
                exit_code: Some(1),
            })
            .unwrap();
        assert_eq!(trace.verificar().unwrap(), 2);
        let entradas = trace.entradas().unwrap();
        assert!(matches!(
            &entradas[0].payload,
            Payload::Intento { numero: 1, .. }
        ));
    }

    #[test]
    fn reabrir_retoma_la_cadena_donde_quedo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trace.jsonl");
        {
            let mut trace = Trace::abrir(&path).unwrap();
            trace.append(payload_creacion(0)).unwrap();
            trace.append(payload_creacion(1)).unwrap();
        }
        let mut trace = Trace::abrir(&path).unwrap();
        trace.append(payload_creacion(2)).unwrap();
        let entradas = trace.entradas().unwrap();
        assert_eq!(entradas.len(), 3);
        assert_eq!(entradas[2].seq, 2);
        assert_eq!(entradas[2].prev_hash, entradas[1].hash);
        assert_eq!(trace.verificar().unwrap(), 3);
    }

    #[test]
    fn alterar_una_entrada_pasada_rompe_la_verificacion() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trace.jsonl");
        let mut trace = Trace::abrir(&path).unwrap();
        for n in 0..3 {
            trace.append(payload_creacion(n)).unwrap();
        }

        // Manipulación externa: se edita el título de la entrada 1.
        let contenido = std::fs::read_to_string(&path).unwrap();
        let alterado = contenido.replace("Requerimiento 1", "Requerimiento X");
        assert_ne!(contenido, alterado, "el reemplazo debe surtir efecto");
        std::fs::write(&path, alterado).unwrap();

        let error = Trace::abrir(&path).unwrap().verificar().unwrap_err();
        assert!(matches!(error, TraceError::HashCorrupto { seq: 1 }));
    }

    #[test]
    fn borrar_una_entrada_intermedia_rompe_la_secuencia() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trace.jsonl");
        let mut trace = Trace::abrir(&path).unwrap();
        for n in 0..3 {
            trace.append(payload_creacion(n)).unwrap();
        }

        let contenido = std::fs::read_to_string(&path).unwrap();
        let sin_linea_1: Vec<&str> = contenido
            .lines()
            .enumerate()
            .filter(|(i, _)| *i != 1)
            .map(|(_, l)| l)
            .collect();
        std::fs::write(&path, sin_linea_1.join("\n")).unwrap();

        let error = Trace::abrir(&path).unwrap().verificar().unwrap_err();
        assert!(matches!(
            error,
            TraceError::SeqInvalida {
                esperada: 1,
                encontrada: 2
            }
        ));
    }

    #[test]
    fn recalcular_el_hash_de_una_entrada_alterada_no_engana_a_la_cadena() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trace.jsonl");
        let mut trace = Trace::abrir(&path).unwrap();
        for n in 0..3 {
            trace.append(payload_creacion(n)).unwrap();
        }

        // El atacante altera la entrada 1 Y recalcula su hash para que la
        // entrada sea internamente consistente. La entrada 2 deja de enlazar.
        let mut entradas = Trace::abrir(&path).unwrap().entradas().unwrap();
        entradas[1].payload = payload_creacion(99);
        entradas[1].hash = entradas[1].calcular_hash().unwrap();
        let reescrito: Vec<String> = entradas
            .iter()
            .map(|e| serde_json::to_string(e).unwrap())
            .collect();
        std::fs::write(&path, reescrito.join("\n")).unwrap();

        let error = Trace::abrir(&path).unwrap().verificar().unwrap_err();
        assert!(matches!(error, TraceError::EnlaceRoto { seq: 2 }));
    }

    #[test]
    fn trace_vacio_verifica_cero_entradas() {
        let (_dir, trace) = trace_temporal();
        assert_eq!(trace.verificar().unwrap(), 0);
    }
}
