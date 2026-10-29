//! Integración con Claude Code en modo no interactivo.
//!
//! Principios (de CLAUDE.md): contexto fresco por intento (spec + resumen
//! estructurado del fallo, nunca transcripciones), escalado de estrategia,
//! prompts de sistema versionados con hash en el trace, presupuestos duros
//! impuestos en Rust y el LLM nunca como compuerta. Este crate solo depende
//! de `core`; el wiring con trace/store/manifest lo hace el binario.

mod bucle;
mod ejecutor;
mod estrategia;
mod gates;
mod presupuesto;
mod prompt;

pub use bucle::{
    Desenlace, InformeEjecucion, IntentoRegistrado, PlanEjecucion, TransicionRegistrada,
    ejecutar_requerimiento,
};
pub use ejecutor::{ClaudeCodeCli, Ejecutor, ResultadoIntento, SolicitudIntento};
pub use estrategia::{Estrategia, MAX_INTENTOS};
pub use gates::{EjecutorGates, GateSpec, GatesEnHost, ResultadoGate};
pub use presupuesto::{ControlPresupuesto, MotivoAgotamiento, Presupuesto};
pub use prompt::{ContextoIntento, MAX_EXTRACTO, PromptVersionado, ResumenFallo, construir_prompt};

/// Errores del agente.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("error de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("respuesta del agente ilegible: {0}")]
    Json(#[from] serde_json::Error),
    #[error("el bucle intentó una transición inválida: {0}")]
    Transicion(#[from] micli_core::TransitionError),
    #[error("no se pudo ejecutar el agente `{binario}`: {fuente}")]
    AgenteNoEjecutable {
        binario: String,
        #[source]
        fuente: std::io::Error,
    },
    /// El proceso del agente salió con error sin producir un resultado
    /// (sesión no iniciada, flag inválido, API caída…). No es un intento:
    /// el bucle lo propaga sin consumir presupuesto ni bloquear.
    #[error(
        "el agente no arrancó (código de salida {}): {stderr}",
        exit_code.map_or_else(|| "desconocido".to_owned(), |c| c.to_string())
    )]
    AgenteNoArranco {
        exit_code: Option<i32>,
        stderr: String,
    },
    #[error("no se pudo ejecutar el gate `{gate}`: {fuente}")]
    GateNoEjecutable {
        gate: String,
        #[source]
        fuente: std::io::Error,
    },
    #[error("no hay estrategia definida para el intento {intento} (invariante rota)")]
    SinEstrategia { intento: u32 },
}
