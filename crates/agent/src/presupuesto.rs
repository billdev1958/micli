//! Presupuestos duros por requerimiento, impuestos en Rust: intentos,
//! tiempo acumulado y tokens. La contabilidad es pura (se alimenta con lo
//! que reporta cada intento), así que es determinista y testeable.

use crate::estrategia::MAX_INTENTOS;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Límites para un requerimiento.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presupuesto {
    /// Se recorta al tope global [`MAX_INTENTOS`].
    pub max_intentos: u32,
    pub max_duracion: Duration,
    /// `None` = sin límite de tokens.
    pub max_tokens: Option<u64>,
}

impl Presupuesto {
    /// Intentos efectivos: nunca más que el tope global.
    pub fn intentos_efectivos(&self) -> u32 {
        self.max_intentos.min(MAX_INTENTOS)
    }
}

/// Por qué se agotó el presupuesto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MotivoAgotamiento {
    Intentos,
    Tiempo,
    Tokens,
}

impl std::fmt::Display for MotivoAgotamiento {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let texto = match self {
            Self::Intentos => "se agotaron los intentos",
            Self::Tiempo => "se agotó el tiempo",
            Self::Tokens => "se agotaron los tokens",
        };
        write!(f, "{texto}")
    }
}

/// Contabilidad acumulada contra un [`Presupuesto`].
#[derive(Debug, Clone)]
pub struct ControlPresupuesto {
    presupuesto: Presupuesto,
    intentos: u32,
    duracion: Duration,
    tokens: u64,
}

impl ControlPresupuesto {
    pub fn nuevo(presupuesto: Presupuesto) -> Self {
        Self {
            presupuesto,
            intentos: 0,
            duracion: Duration::ZERO,
            tokens: 0,
        }
    }

    /// Registra lo consumido por un intento terminado.
    pub fn registrar_intento(&mut self, duracion: Duration, tokens: u64) {
        self.intentos += 1;
        self.duracion = self.duracion.saturating_add(duracion);
        self.tokens = self.tokens.saturating_add(tokens);
    }

    /// ¿Hay presupuesto para un intento más? Devuelve el primer motivo de
    /// agotamiento encontrado, en orden: intentos, tiempo, tokens.
    pub fn permite_otro_intento(&self) -> Result<(), MotivoAgotamiento> {
        if self.intentos >= self.presupuesto.intentos_efectivos() {
            return Err(MotivoAgotamiento::Intentos);
        }
        if self.duracion >= self.presupuesto.max_duracion {
            return Err(MotivoAgotamiento::Tiempo);
        }
        if let Some(max) = self.presupuesto.max_tokens
            && self.tokens >= max
        {
            return Err(MotivoAgotamiento::Tokens);
        }
        Ok(())
    }

    pub fn intentos(&self) -> u32 {
        self.intentos
    }

    pub fn tokens(&self) -> u64 {
        self.tokens
    }

    pub fn duracion(&self) -> Duration {
        self.duracion
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn presupuesto(intentos: u32, minutos: u64, tokens: Option<u64>) -> Presupuesto {
        Presupuesto {
            max_intentos: intentos,
            max_duracion: Duration::from_secs(minutos * 60),
            max_tokens: tokens,
        }
    }

    #[test]
    fn el_tope_global_de_intentos_no_es_negociable() {
        assert_eq!(presupuesto(99, 30, None).intentos_efectivos(), MAX_INTENTOS);
        assert_eq!(presupuesto(3, 30, None).intentos_efectivos(), 3);
    }

    #[test]
    fn agota_por_intentos() {
        let mut control = ControlPresupuesto::nuevo(presupuesto(2, 30, None));
        assert!(control.permite_otro_intento().is_ok());
        control.registrar_intento(Duration::from_secs(1), 10);
        assert!(control.permite_otro_intento().is_ok());
        control.registrar_intento(Duration::from_secs(1), 10);
        assert_eq!(
            control.permite_otro_intento(),
            Err(MotivoAgotamiento::Intentos)
        );
    }

    #[test]
    fn agota_por_tiempo_acumulado() {
        let mut control = ControlPresupuesto::nuevo(presupuesto(5, 1, None));
        control.registrar_intento(Duration::from_secs(61), 0);
        assert_eq!(
            control.permite_otro_intento(),
            Err(MotivoAgotamiento::Tiempo)
        );
    }

    #[test]
    fn agota_por_tokens_solo_si_hay_limite() {
        let mut con_limite = ControlPresupuesto::nuevo(presupuesto(5, 30, Some(100)));
        con_limite.registrar_intento(Duration::from_secs(1), 100);
        assert_eq!(
            con_limite.permite_otro_intento(),
            Err(MotivoAgotamiento::Tokens)
        );

        let mut sin_limite = ControlPresupuesto::nuevo(presupuesto(5, 30, None));
        sin_limite.registrar_intento(Duration::from_secs(1), u64::MAX);
        assert!(sin_limite.permite_otro_intento().is_ok());
    }
}
