//! Tests del bucle de intentos con ejecutor y gates simulados, más
//! propiedades: el bucle siempre termina, nunca excede el tope de intentos y
//! toda transición registrada es válida según `core`.
//!
//! `allow-unwrap-in-tests` de clippy.toml solo cubre funciones `#[test]`;
//! los helpers y mocks de este archivo también son código de test.
#![allow(clippy::unwrap_used)]

use micli_agent::{
    AgentError, Desenlace, Ejecutor, EjecutorGates, Estrategia, GateSpec, InformeEjecucion,
    PlanEjecucion, Presupuesto, PromptVersionado, ResultadoGate, ResultadoIntento,
    SolicitudIntento, ejecutar_requerimiento,
};
use micli_core::{ActiveState, State, transition};
use proptest::prelude::*;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Ejecutor simulado: registra los prompts recibidos y responde según una
/// secuencia configurada (true = el agente terminó bien).
struct EjecutorMock {
    exitos: Vec<bool>,
    tokens_por_intento: u64,
    prompts: Vec<String>,
}

impl EjecutorMock {
    fn nuevo(exitos: Vec<bool>) -> Self {
        Self {
            exitos,
            tokens_por_intento: 100,
            prompts: Vec::new(),
        }
    }
}

impl Ejecutor for EjecutorMock {
    fn ejecutar(
        &mut self,
        solicitud: SolicitudIntento<'_>,
    ) -> Result<ResultadoIntento, AgentError> {
        let indice = self.prompts.len();
        self.prompts.push(solicitud.prompt);
        Ok(ResultadoIntento {
            exito: self.exitos.get(indice).copied().unwrap_or(true),
            tokens: self.tokens_por_intento,
            duracion: Duration::from_secs(1),
        })
    }
}

/// Gates simulados: el intento N pasa o falla según `patron[N]`; cuenta
/// cuántas rondas de verificación hubo.
struct GatesMock {
    patron: Vec<bool>,
    ronda: usize,
    llamadas: u32,
}

impl GatesMock {
    fn nuevo(patron: Vec<bool>) -> Self {
        Self {
            patron,
            ronda: 0,
            llamadas: 0,
        }
    }
}

impl EjecutorGates for GatesMock {
    fn ejecutar(&mut self, gate: &GateSpec, _dir: &Path) -> Result<ResultadoGate, AgentError> {
        self.llamadas += 1;
        let paso = self.patron.get(self.ronda).copied().unwrap_or(false);
        self.ronda += 1;
        Ok(ResultadoGate {
            gate: gate.name.clone(),
            paso,
            exit_code: Some(i32::from(!paso)),
            salida: if paso {
                "ok".to_owned()
            } else {
                "assert fallo en linea 42".to_owned()
            },
        })
    }
}

fn gate_requerido() -> Vec<GateSpec> {
    vec![GateSpec {
        name: "tests".to_owned(),
        cmd: "echo tests".to_owned(),
        required: true,
    }]
}

fn presupuesto_amplio() -> Presupuesto {
    Presupuesto {
        max_intentos: 5,
        max_duracion: Duration::from_secs(3600),
        max_tokens: None,
    }
}

fn correr(
    ejecutor: &mut EjecutorMock,
    gates: &mut GatesMock,
    gates_spec: &[GateSpec],
    presupuesto: Presupuesto,
) -> InformeEjecucion {
    let sistema = PromptVersionado::desde_texto("prompt de sistema".to_owned());
    let dir = PathBuf::from(".");
    let plan = PlanEjecucion {
        spec: "implementar suma",
        prompt_sistema: &sistema,
        gates: gates_spec,
        presupuesto,
        dir_trabajo: &dir,
    };
    ejecutar_requerimiento(&plan, ejecutor, gates).unwrap()
}

#[test]
fn acepta_al_primer_intento_si_los_gates_pasan() {
    let mut ejecutor = EjecutorMock::nuevo(vec![true]);
    let mut gates = GatesMock::nuevo(vec![true]);
    let informe = correr(
        &mut ejecutor,
        &mut gates,
        &gate_requerido(),
        presupuesto_amplio(),
    );

    assert_eq!(informe.desenlace, Desenlace::Aceptado);
    assert_eq!(informe.estado_final, State::Aceptado);
    assert_eq!(informe.intentos.len(), 1);
    assert_eq!(informe.intentos[0].estrategia, Estrategia::Directo);
    let eventos: Vec<_> = informe.transiciones.iter().map(|t| t.evento).collect();
    assert_eq!(
        eventos,
        vec![
            micli_core::Event::IniciarImplementacion,
            micli_core::Event::EnviarAVerificacion,
            micli_core::Event::AprobarGates,
        ]
    );
}

#[test]
fn reintenta_con_feedback_y_acepta_al_tercero() {
    let mut ejecutor = EjecutorMock::nuevo(vec![true, true, true]);
    let mut gates = GatesMock::nuevo(vec![false, false, true]);
    let informe = correr(
        &mut ejecutor,
        &mut gates,
        &gate_requerido(),
        presupuesto_amplio(),
    );

    assert_eq!(informe.desenlace, Desenlace::Aceptado);
    assert_eq!(informe.intentos.len(), 3);
    assert_eq!(informe.intentos[1].estrategia, Estrategia::ConFeedback);
    // El intento 2 ve el resumen estructurado del fallo, no la transcripción.
    assert!(ejecutor.prompts[1].contains("assert fallo en linea 42"));
    assert!(ejecutor.prompts[1].contains("Gate `tests`"));
    // El intento 1 no lleva feedback.
    assert!(!ejecutor.prompts[0].contains("Fallo del intento anterior"));
}

#[test]
fn tras_cinco_intentos_fallidos_bloquea_con_resumen() {
    let mut ejecutor = EjecutorMock::nuevo(vec![true; 5]);
    let mut gates = GatesMock::nuevo(vec![false; 5]);
    let informe = correr(
        &mut ejecutor,
        &mut gates,
        &gate_requerido(),
        presupuesto_amplio(),
    );

    match &informe.desenlace {
        Desenlace::Bloqueado { motivo } => {
            assert!(motivo.contains("intentos"), "{motivo}");
            assert!(motivo.contains("gate `tests`"), "{motivo}");
        }
        otro => panic!("esperaba Bloqueado, fue {otro:?}"),
    }
    assert_eq!(informe.intentos.len(), 5);
    assert_eq!(
        informe.estado_final,
        State::Bloqueado {
            desde: ActiveState::Implementando
        }
    );
    // La escalada completa: Directo, ConFeedback x2, ContextoLimpio, Diagnostico.
    let estrategias: Vec<_> = informe.intentos.iter().map(|i| i.estrategia).collect();
    assert_eq!(
        estrategias,
        vec![
            Estrategia::Directo,
            Estrategia::ConFeedback,
            Estrategia::ConFeedback,
            Estrategia::ContextoLimpioConHipotesis,
            Estrategia::DiagnosticoPrevio,
        ]
    );
    // El intento 4 (contexto limpio) NO ve el fallo; el 5 (diagnóstico) sí.
    assert!(!ejecutor.prompts[3].contains("Fallo del intento anterior"));
    assert!(ejecutor.prompts[4].contains("Fallo del intento anterior"));
}

#[test]
fn el_presupuesto_de_tokens_bloquea_aunque_queden_intentos() {
    let mut ejecutor = EjecutorMock::nuevo(vec![true; 5]);
    ejecutor.tokens_por_intento = 1000;
    let mut gates = GatesMock::nuevo(vec![false; 5]);
    let presupuesto = Presupuesto {
        max_intentos: 5,
        max_duracion: Duration::from_secs(3600),
        max_tokens: Some(1000),
    };
    let informe = correr(&mut ejecutor, &mut gates, &gate_requerido(), presupuesto);

    assert_eq!(informe.intentos.len(), 1);
    match &informe.desenlace {
        Desenlace::Bloqueado { motivo } => assert!(motivo.contains("tokens"), "{motivo}"),
        otro => panic!("esperaba Bloqueado, fue {otro:?}"),
    }
}

#[test]
fn si_el_agente_no_termina_cuenta_intento_y_no_corre_gates() {
    let mut ejecutor = EjecutorMock::nuevo(vec![false, true]);
    let mut gates = GatesMock::nuevo(vec![true]);
    let informe = correr(
        &mut ejecutor,
        &mut gates,
        &gate_requerido(),
        presupuesto_amplio(),
    );

    assert_eq!(informe.desenlace, Desenlace::Aceptado);
    assert_eq!(informe.intentos.len(), 2);
    assert!(informe.intentos[0].gates.is_empty());
    assert_eq!(gates.llamadas, 1);
}

/// Ejecutor cuyo proceso nunca arranca (p. ej. `claude` sin sesión).
struct EjecutorQueNoArranca {
    llamadas: u32,
}

impl Ejecutor for EjecutorQueNoArranca {
    fn ejecutar(
        &mut self,
        _solicitud: SolicitudIntento<'_>,
    ) -> Result<ResultadoIntento, AgentError> {
        self.llamadas += 1;
        Err(AgentError::AgenteNoArranco {
            exit_code: Some(1),
            stderr: "Not logged in".to_owned(),
        })
    }
}

#[test]
fn si_el_agente_no_arranca_no_consume_intentos_ni_bloquea() {
    let mut ejecutor = EjecutorQueNoArranca { llamadas: 0 };
    let mut gates = GatesMock::nuevo(vec![true]);
    let sistema = PromptVersionado::desde_texto("sistema".to_owned());
    let gates_spec = gate_requerido();
    let dir = PathBuf::from(".");
    let plan = PlanEjecucion {
        spec: "implementar suma",
        prompt_sistema: &sistema,
        gates: &gates_spec,
        presupuesto: presupuesto_amplio(),
        dir_trabajo: &dir,
    };

    let error = ejecutar_requerimiento(&plan, &mut ejecutor, &mut gates).unwrap_err();
    assert!(
        matches!(
            &error,
            AgentError::AgenteNoArranco {
                exit_code: Some(1),
                stderr
            } if stderr == "Not logged in"
        ),
        "{error:?}"
    );
    // Una sola invocación: sin reintentos ciegos ni escalado de estrategia.
    assert_eq!(ejecutor.llamadas, 1);
    assert_eq!(gates.llamadas, 0);
    assert!(error.to_string().contains("Not logged in"), "{error}");
}

#[test]
fn un_gate_no_requerido_que_falla_no_impide_aceptar() {
    let gates_spec = vec![
        GateSpec {
            name: "tests".to_owned(),
            cmd: "echo".to_owned(),
            required: true,
        },
        GateSpec {
            name: "advisory".to_owned(),
            cmd: "echo".to_owned(),
            required: false,
        },
    ];
    // Ronda única: tests pasa, advisory falla.
    let mut ejecutor = EjecutorMock::nuevo(vec![true]);
    let mut gates = GatesMock::nuevo(vec![true, false]);
    let informe = correr(&mut ejecutor, &mut gates, &gates_spec, presupuesto_amplio());

    assert_eq!(informe.desenlace, Desenlace::Aceptado);
    assert_eq!(informe.intentos[0].gates.len(), 2);
    assert!(!informe.intentos[0].gates[1].paso);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Para cualquier patrón de resultados de agente/gates, el bucle termina,
    /// respeta el tope de intentos, su desenlace es coherente y cada
    /// transición registrada es exactamente la que `core` habría producido.
    #[test]
    fn el_bucle_siempre_termina_y_respeta_el_diagrama(
        exitos_agente in prop::collection::vec(any::<bool>(), 5),
        gates_pasan in prop::collection::vec(any::<bool>(), 5),
        max_intentos in 1u32..=5,
    ) {
        let mut ejecutor = EjecutorMock::nuevo(exitos_agente);
        let mut gates = GatesMock::nuevo(gates_pasan);
        let presupuesto = Presupuesto {
            max_intentos,
            max_duracion: Duration::from_secs(3600),
            max_tokens: None,
        };
        let informe = correr(&mut ejecutor, &mut gates, &gate_requerido(), presupuesto);

        prop_assert!(informe.intentos.len() as u32 <= max_intentos);
        let acepto = informe.desenlace == Desenlace::Aceptado;
        prop_assert_eq!(acepto, informe.intentos.iter().any(|i| i.gates_pasaron));
        let termino_en_estado_final = matches!(
            informe.estado_final,
            State::Aceptado | State::Bloqueado { .. }
        );
        prop_assert!(termino_en_estado_final);

        // Re-aplicar cada transición contra core desde TestRojo.
        let mut estado = State::TestRojo;
        for t in &informe.transiciones {
            prop_assert_eq!(t.desde, estado);
            estado = transition(t.desde, t.evento).unwrap();
            prop_assert_eq!(t.hacia, estado);
        }
        prop_assert_eq!(estado, informe.estado_final);
    }
}
