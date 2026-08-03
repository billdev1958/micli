//! Escalado de estrategia entre intentos. El cumplimiento del tope es código
//! Rust determinista: el agente no puede pedir un sexto intento.

use serde::{Deserialize, Serialize};

/// Tope duro de intentos por requerimiento. Después de esto el requerimiento
/// pasa a `Bloqueado` con resumen de lo intentado.
pub const MAX_INTENTOS: u32 = 5;

/// Cómo se plantea cada intento. El intento N siempre parte de contexto
/// fresco: spec + resumen estructurado del fallo anterior, nunca la
/// transcripción completa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Estrategia {
    /// Intento 1: la spec tal cual.
    Directo,
    /// Intentos 2 y 3: spec + resumen del fallo del intento anterior.
    ConFeedback,
    /// Intento 4: contexto limpio; se le pide formular una hipótesis del
    /// fallo antes de implementar.
    ContextoLimpioConHipotesis,
    /// Intento 5: diagnóstico del problema antes de tocar código.
    DiagnosticoPrevio,
}

impl Estrategia {
    /// Estrategia para el intento `numero` (1-indexado). `None` si el número
    /// está fuera del rango permitido.
    pub fn para_intento(numero: u32) -> Option<Self> {
        match numero {
            1 => Some(Self::Directo),
            2 | 3 => Some(Self::ConFeedback),
            4 => Some(Self::ContextoLimpioConHipotesis),
            5 => Some(Self::DiagnosticoPrevio),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escala_como_documenta_claude_md() {
        assert_eq!(Estrategia::para_intento(1), Some(Estrategia::Directo));
        assert_eq!(Estrategia::para_intento(2), Some(Estrategia::ConFeedback));
        assert_eq!(Estrategia::para_intento(3), Some(Estrategia::ConFeedback));
        assert_eq!(
            Estrategia::para_intento(4),
            Some(Estrategia::ContextoLimpioConHipotesis)
        );
        assert_eq!(
            Estrategia::para_intento(5),
            Some(Estrategia::DiagnosticoPrevio)
        );
    }

    #[test]
    fn fuera_de_rango_no_hay_estrategia() {
        assert_eq!(Estrategia::para_intento(0), None);
        assert_eq!(Estrategia::para_intento(MAX_INTENTOS + 1), None);
        assert_eq!(Estrategia::para_intento(u32::MAX), None);
    }
}
