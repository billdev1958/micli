//! Invariantes de la máquina de estados verificadas con property-based
//! testing: se aplican secuencias arbitrarias de eventos (ignorando las
//! transiciones inválidas, que dejan el estado intacto) y se comprueba que
//! ninguna secuencia viole el diagrama.

use micli_core::{ActiveState, Event, State, transition};
use proptest::prelude::*;

fn evento_arbitrario() -> impl Strategy<Value = Event> {
    prop::sample::select(Event::TODOS.to_vec())
}

fn estado_activo_arbitrario() -> impl Strategy<Value = ActiveState> {
    prop::sample::select(ActiveState::TODOS.to_vec())
}

proptest! {
    /// Ninguna secuencia de eventos llega a `Aceptado` sin pasar por
    /// `TestRojo`.
    #[test]
    fn aceptado_requiere_haber_pasado_por_test_rojo(
        eventos in prop::collection::vec(evento_arbitrario(), 0..64),
    ) {
        let mut estado = State::Draft;
        let mut paso_por_test_rojo = false;
        for evento in eventos {
            if let Ok(nuevo) = transition(estado, evento) {
                estado = nuevo;
                if estado == State::TestRojo {
                    paso_por_test_rojo = true;
                }
            }
        }
        if estado == State::Aceptado {
            prop_assert!(paso_por_test_rojo);
        }
    }

    /// `Aceptado` es terminal: una vez alcanzado, ningún evento posterior
    /// produce una transición válida.
    #[test]
    fn aceptado_es_terminal(
        eventos in prop::collection::vec(evento_arbitrario(), 0..64),
    ) {
        let mut estado = State::Draft;
        for evento in eventos {
            if estado == State::Aceptado {
                prop_assert!(transition(estado, evento).is_err());
            } else if let Ok(nuevo) = transition(estado, evento) {
                estado = nuevo;
            }
        }
    }

    /// Bloquear y desbloquear regresa exactamente al estado original.
    #[test]
    fn desbloquear_regresa_al_estado_previo(activo in estado_activo_arbitrario()) {
        let original = State::from(activo);
        let bloqueado = transition(original, Event::Bloquear)?;
        prop_assert_eq!(bloqueado, State::Bloqueado { desde: activo });
        prop_assert_eq!(transition(bloqueado, Event::Desbloquear)?, original);
    }

    /// Todo estado alcanzable no terminal ni bloqueado admite `Bloquear`, y
    /// `Bloqueado` solo admite `Desbloquear`.
    #[test]
    fn bloqueado_solo_admite_desbloquear(
        activo in estado_activo_arbitrario(),
        evento in evento_arbitrario(),
    ) {
        let bloqueado = State::Bloqueado { desde: activo };
        let resultado = transition(bloqueado, evento);
        if evento == Event::Desbloquear {
            prop_assert_eq!(resultado, Ok(State::from(activo)));
        } else {
            prop_assert!(resultado.is_err());
        }
    }
}
