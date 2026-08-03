//! Máquina de estados de un requerimiento.
//!
//! Las transiciones viven únicamente aquí, vía [`transition()`]. Ningún otro
//! módulo del workspace muta estado directamente.
//!
//! ```text
//! Draft → SpecAprobada → TestRojo → Implementando → Verificando → Aceptado
//!                                         ↑               │
//!                                         └─ (gate falla) ─┘
//! Bloqueado: alcanzable desde cualquier estado activo; Desbloquear regresa
//! al estado desde el que se bloqueó.
//! ```

use serde::{Deserialize, Serialize};
use std::fmt;

/// Estados activos: aquellos desde los que un requerimiento puede
/// bloquearse. `Aceptado` es terminal y `Bloqueado` ya está bloqueado, por
/// eso no aparecen aquí.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActiveState {
    Draft,
    SpecAprobada,
    TestRojo,
    Implementando,
    Verificando,
}

impl ActiveState {
    /// Todos los estados activos, para iterar en tests e invariantes.
    pub const TODOS: [Self; 5] = [
        Self::Draft,
        Self::SpecAprobada,
        Self::TestRojo,
        Self::Implementando,
        Self::Verificando,
    ];
}

/// Estado de un requerimiento.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum State {
    Draft,
    SpecAprobada,
    TestRojo,
    Implementando,
    Verificando,
    /// Terminal: no admite ninguna transición saliente.
    Aceptado,
    /// Requiere intervención humana. Guarda el estado activo desde el que se
    /// bloqueó para poder regresar a él al desbloquear.
    Bloqueado {
        desde: ActiveState,
    },
}

impl State {
    /// El estado como activo, o `None` si es `Aceptado` o `Bloqueado`.
    pub fn como_activo(self) -> Option<ActiveState> {
        match self {
            Self::Draft => Some(ActiveState::Draft),
            Self::SpecAprobada => Some(ActiveState::SpecAprobada),
            Self::TestRojo => Some(ActiveState::TestRojo),
            Self::Implementando => Some(ActiveState::Implementando),
            Self::Verificando => Some(ActiveState::Verificando),
            Self::Aceptado | Self::Bloqueado { .. } => None,
        }
    }
}

impl From<ActiveState> for State {
    fn from(activo: ActiveState) -> Self {
        match activo {
            ActiveState::Draft => Self::Draft,
            ActiveState::SpecAprobada => Self::SpecAprobada,
            ActiveState::TestRojo => Self::TestRojo,
            ActiveState::Implementando => Self::Implementando,
            ActiveState::Verificando => Self::Verificando,
        }
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Draft => write!(f, "Draft"),
            Self::SpecAprobada => write!(f, "SpecAprobada"),
            Self::TestRojo => write!(f, "TestRojo"),
            Self::Implementando => write!(f, "Implementando"),
            Self::Verificando => write!(f, "Verificando"),
            Self::Aceptado => write!(f, "Aceptado"),
            Self::Bloqueado { desde } => write!(f, "Bloqueado(desde {})", State::from(*desde)),
        }
    }
}

/// Eventos que disparan transiciones. El contexto de cada evento (motivo del
/// bloqueo, resultado de gates, etc.) pertenece al trace (Fase 2), no a la
/// máquina de estados.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Event {
    /// `Draft → SpecAprobada`
    AprobarSpec,
    /// `SpecAprobada → TestRojo`: se confirmó que existe un test en rojo.
    ConfirmarTestRojo,
    /// `TestRojo → Implementando`
    IniciarImplementacion,
    /// `Implementando → Verificando`
    EnviarAVerificacion,
    /// `Verificando → Aceptado`: todos los gates requeridos pasaron.
    AprobarGates,
    /// `Verificando → Implementando`: algún gate requerido falló.
    FallarGate,
    /// `<activo> → Bloqueado`
    Bloquear,
    /// `Bloqueado → <estado desde el que se bloqueó>`
    Desbloquear,
}

impl Event {
    /// Todos los eventos, para iterar en tests e invariantes.
    pub const TODOS: [Self; 8] = [
        Self::AprobarSpec,
        Self::ConfirmarTestRojo,
        Self::IniciarImplementacion,
        Self::EnviarAVerificacion,
        Self::AprobarGates,
        Self::FallarGate,
        Self::Bloquear,
        Self::Desbloquear,
    ];
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let nombre = match self {
            Self::AprobarSpec => "AprobarSpec",
            Self::ConfirmarTestRojo => "ConfirmarTestRojo",
            Self::IniciarImplementacion => "IniciarImplementacion",
            Self::EnviarAVerificacion => "EnviarAVerificacion",
            Self::AprobarGates => "AprobarGates",
            Self::FallarGate => "FallarGate",
            Self::Bloquear => "Bloquear",
            Self::Desbloquear => "Desbloquear",
        };
        write!(f, "{nombre}")
    }
}

/// Error al intentar una transición no permitida.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TransitionError {
    #[error("transición inválida: el evento `{evento}` no aplica en el estado `{desde}`")]
    Invalida { desde: State, evento: Event },
}

/// Aplica `evento` sobre `desde` y devuelve el nuevo estado, o error si la
/// transición no está permitida por el diagrama.
pub fn transition(desde: State, evento: Event) -> Result<State, TransitionError> {
    let hacia = match (desde, evento) {
        (State::Draft, Event::AprobarSpec) => State::SpecAprobada,
        (State::SpecAprobada, Event::ConfirmarTestRojo) => State::TestRojo,
        (State::TestRojo, Event::IniciarImplementacion) => State::Implementando,
        (State::Implementando, Event::EnviarAVerificacion) => State::Verificando,
        (State::Verificando, Event::AprobarGates) => State::Aceptado,
        (State::Verificando, Event::FallarGate) => State::Implementando,
        (State::Bloqueado { desde }, Event::Desbloquear) => State::from(desde),
        (estado, Event::Bloquear) => match estado.como_activo() {
            Some(activo) => State::Bloqueado { desde: activo },
            None => return Err(TransitionError::Invalida { desde, evento }),
        },
        _ => return Err(TransitionError::Invalida { desde, evento }),
    };
    Ok(hacia)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La tabla completa de transiciones válidas del diagrama. Cualquier par
    /// (estado, evento) fuera de esta tabla debe ser rechazado.
    fn tabla_valida() -> Vec<(State, Event, State)> {
        let mut tabla = vec![
            (State::Draft, Event::AprobarSpec, State::SpecAprobada),
            (
                State::SpecAprobada,
                Event::ConfirmarTestRojo,
                State::TestRojo,
            ),
            (
                State::TestRojo,
                Event::IniciarImplementacion,
                State::Implementando,
            ),
            (
                State::Implementando,
                Event::EnviarAVerificacion,
                State::Verificando,
            ),
            (State::Verificando, Event::AprobarGates, State::Aceptado),
            (State::Verificando, Event::FallarGate, State::Implementando),
        ];
        for activo in ActiveState::TODOS {
            tabla.push((
                State::from(activo),
                Event::Bloquear,
                State::Bloqueado { desde: activo },
            ));
            tabla.push((
                State::Bloqueado { desde: activo },
                Event::Desbloquear,
                State::from(activo),
            ));
        }
        tabla
    }

    fn todos_los_estados() -> Vec<State> {
        let mut estados = vec![State::Aceptado];
        for activo in ActiveState::TODOS {
            estados.push(State::from(activo));
            estados.push(State::Bloqueado { desde: activo });
        }
        estados
    }

    #[test]
    fn acepta_exactamente_las_transiciones_del_diagrama() {
        for (desde, evento, hacia) in tabla_valida() {
            assert_eq!(
                transition(desde, evento),
                Ok(hacia),
                "esperaba {desde} --{evento}--> {hacia}"
            );
        }
    }

    #[test]
    fn rechaza_todo_par_fuera_de_la_tabla() {
        let tabla = tabla_valida();
        for desde in todos_los_estados() {
            for evento in Event::TODOS {
                if tabla.iter().any(|(d, e, _)| *d == desde && *e == evento) {
                    continue;
                }
                assert_eq!(
                    transition(desde, evento),
                    Err(TransitionError::Invalida { desde, evento }),
                    "no esperaba que {desde} --{evento}--> fuera válida"
                );
            }
        }
    }

    #[test]
    fn aceptado_es_terminal() {
        for evento in Event::TODOS {
            assert!(transition(State::Aceptado, evento).is_err());
        }
    }
}
