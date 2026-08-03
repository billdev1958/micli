//! Modelo de un requerimiento. El estado solo cambia vía [`Requirement::aplicar`],
//! que delega en [`crate::state::transition`].

use crate::state::{Event, State, TransitionError, transition};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Identificador opaco de un requerimiento dentro de un proyecto.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequirementId(String);

impl RequirementId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RequirementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Un requerimiento y su estado actual. Nace en `Draft`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requirement {
    /// Versión del esquema serializado, para migraciones futuras.
    pub version: u32,
    pub id: RequirementId,
    pub titulo: String,
    pub estado: State,
}

impl Requirement {
    /// Versión actual del esquema serializado.
    pub const VERSION: u32 = 1;

    pub fn nuevo(id: RequirementId, titulo: impl Into<String>) -> Self {
        Self {
            version: Self::VERSION,
            id,
            titulo: titulo.into(),
            estado: State::Draft,
        }
    }

    /// Aplica un evento al requerimiento. Si la transición es válida, muta el
    /// estado y lo devuelve; si no, el requerimiento queda intacto.
    pub fn aplicar(&mut self, evento: Event) -> Result<State, TransitionError> {
        self.estado = transition(self.estado, evento)?;
        Ok(self.estado)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn requerimiento() -> Requirement {
        Requirement::nuevo(RequirementId::new("REQ-001"), "Alta de usuarios")
    }

    #[test]
    fn nace_en_draft_con_version_actual() {
        let req = requerimiento();
        assert_eq!(req.estado, State::Draft);
        assert_eq!(req.version, Requirement::VERSION);
    }

    #[test]
    fn aplicar_avanza_el_ciclo_completo_hasta_aceptado() {
        let mut req = requerimiento();
        for evento in [
            Event::AprobarSpec,
            Event::ConfirmarTestRojo,
            Event::IniciarImplementacion,
            Event::EnviarAVerificacion,
            Event::AprobarGates,
        ] {
            req.aplicar(evento).unwrap();
        }
        assert_eq!(req.estado, State::Aceptado);
    }

    #[test]
    fn aplicar_invalido_no_muta_el_estado() {
        let mut req = requerimiento();
        let error = req.aplicar(Event::AprobarGates).unwrap_err();
        assert_eq!(
            error,
            TransitionError::Invalida {
                desde: State::Draft,
                evento: Event::AprobarGates,
            }
        );
        assert_eq!(req.estado, State::Draft);
    }

    #[test]
    fn serde_roundtrip_preserva_el_requerimiento() {
        let mut req = requerimiento();
        req.aplicar(Event::AprobarSpec).unwrap();
        req.aplicar(Event::Bloquear).unwrap();

        let json = serde_json::to_string(&req).unwrap();
        let leido: Requirement = serde_json::from_str(&json).unwrap();
        assert_eq!(leido, req);
    }

    #[test]
    fn serde_rechaza_campos_desconocidos() {
        let json = r#"{
            "version": 1,
            "id": "REQ-001",
            "titulo": "Alta de usuarios",
            "estado": "Draft",
            "intruso": true
        }"#;
        assert!(serde_json::from_str::<Requirement>(json).is_err());
    }
}
