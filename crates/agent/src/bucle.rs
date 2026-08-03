//! El bucle de intentos: orquesta agente y gates contra la máquina de
//! estados de `core`. Toma control con el requerimiento en `TestRojo` y
//! termina en `Aceptado` o `Bloqueado`; cada cambio de estado pasa por
//! `core::transition()`, así que violar el diagrama no compila... o no corre.

use crate::AgentError;
use crate::ejecutor::{Ejecutor, SolicitudIntento};
use crate::estrategia::Estrategia;
use crate::gates::{EjecutorGates, GateSpec, ResultadoGate};
use crate::presupuesto::{ControlPresupuesto, Presupuesto};
use crate::prompt::{ContextoIntento, PromptVersionado, ResumenFallo, construir_prompt};
use micli_core::{Event, State, transition};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

/// Todo lo que necesita el bucle para un requerimiento.
#[derive(Debug)]
pub struct PlanEjecucion<'a> {
    pub spec: &'a str,
    pub prompt_sistema: &'a PromptVersionado,
    pub gates: &'a [GateSpec],
    pub presupuesto: Presupuesto,
    pub dir_trabajo: &'a Path,
}

/// Una transición ya validada, lista para persistirse en el trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransicionRegistrada {
    pub evento: Event,
    pub desde: State,
    pub hacia: State,
}

/// Lo ocurrido en un intento, para el trace y la proyección.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentoRegistrado {
    pub numero: u32,
    pub estrategia: Estrategia,
    pub tokens: u64,
    pub duracion: Duration,
    /// Resultados de todos los gates (también los no requeridos). Vacío si
    /// el agente no terminó su propia ejecución.
    pub gates: Vec<ResultadoGate>,
    pub gates_pasaron: bool,
}

/// Cómo terminó el requerimiento.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Desenlace {
    Aceptado,
    /// Requiere intervención humana; `motivo` resume qué se intentó.
    Bloqueado {
        motivo: String,
    },
}

/// Informe completo para que `micli` lo persista (trace, store, terminal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InformeEjecucion {
    pub desenlace: Desenlace,
    pub intentos: Vec<IntentoRegistrado>,
    pub transiciones: Vec<TransicionRegistrada>,
    pub hash_prompt_sistema: String,
    pub estado_final: State,
}

fn aplicar(
    estado: &mut State,
    transiciones: &mut Vec<TransicionRegistrada>,
    evento: Event,
) -> Result<(), AgentError> {
    let desde = *estado;
    let hacia = transition(desde, evento)?;
    transiciones.push(TransicionRegistrada {
        evento,
        desde,
        hacia,
    });
    *estado = hacia;
    Ok(())
}

/// Ejecuta el ciclo completo de un requerimiento que ya está en `TestRojo`.
pub fn ejecutar_requerimiento(
    plan: &PlanEjecucion<'_>,
    ejecutor: &mut dyn Ejecutor,
    gates: &mut dyn EjecutorGates,
) -> Result<InformeEjecucion, AgentError> {
    let mut estado = State::TestRojo;
    let mut transiciones = Vec::new();
    let mut intentos: Vec<IntentoRegistrado> = Vec::new();
    let mut control = ControlPresupuesto::nuevo(plan.presupuesto);
    let mut fallo_anterior: Option<ResumenFallo> = None;

    aplicar(&mut estado, &mut transiciones, Event::IniciarImplementacion)?;

    loop {
        if let Err(motivo) = control.permite_otro_intento() {
            aplicar(&mut estado, &mut transiciones, Event::Bloquear)?;
            let ultimo = fallo_anterior.as_ref().map_or_else(String::new, |f| {
                format!("; último fallo: gate `{}`", f.gate)
            });
            return Ok(InformeEjecucion {
                desenlace: Desenlace::Bloqueado {
                    motivo: format!("{motivo} tras {} intento(s){ultimo}", control.intentos()),
                },
                intentos,
                transiciones,
                hash_prompt_sistema: plan.prompt_sistema.hash().to_owned(),
                estado_final: estado,
            });
        }

        let numero = control.intentos() + 1;
        let estrategia = Estrategia::para_intento(numero)
            .ok_or(AgentError::SinEstrategia { intento: numero })?;
        // Contexto fresco: solo estas estrategias ven el resumen del fallo.
        let feedback = match estrategia {
            Estrategia::ConFeedback | Estrategia::DiagnosticoPrevio => fallo_anterior.as_ref(),
            Estrategia::Directo | Estrategia::ContextoLimpioConHipotesis => None,
        };
        let prompt = construir_prompt(ContextoIntento {
            spec: plan.spec,
            numero,
            estrategia,
            fallo_anterior: feedback,
        });

        let resultado = ejecutor.ejecutar(SolicitudIntento {
            prompt_sistema: plan.prompt_sistema,
            prompt,
            dir_trabajo: plan.dir_trabajo,
        })?;
        control.registrar_intento(resultado.duracion, resultado.tokens);

        if !resultado.exito {
            // El agente no terminó: cuenta como intento fallido sin gates.
            fallo_anterior = Some(ResumenFallo::nuevo(
                "agente",
                None,
                "el agente no terminó su ejecución",
            ));
            intentos.push(IntentoRegistrado {
                numero,
                estrategia,
                tokens: resultado.tokens,
                duracion: resultado.duracion,
                gates: Vec::new(),
                gates_pasaron: false,
            });
            continue;
        }

        aplicar(&mut estado, &mut transiciones, Event::EnviarAVerificacion)?;
        let mut resultados = Vec::new();
        let mut primer_fallo: Option<ResumenFallo> = None;
        for gate in plan.gates {
            let r = gates.ejecutar(gate, plan.dir_trabajo)?;
            if gate.required && !r.paso && primer_fallo.is_none() {
                primer_fallo = Some(ResumenFallo::nuevo(&*r.gate, r.exit_code, &r.salida));
            }
            resultados.push(r);
        }
        let gates_pasaron = primer_fallo.is_none();
        intentos.push(IntentoRegistrado {
            numero,
            estrategia,
            tokens: resultado.tokens,
            duracion: resultado.duracion,
            gates: resultados,
            gates_pasaron,
        });

        if gates_pasaron {
            aplicar(&mut estado, &mut transiciones, Event::AprobarGates)?;
            return Ok(InformeEjecucion {
                desenlace: Desenlace::Aceptado,
                intentos,
                transiciones,
                hash_prompt_sistema: plan.prompt_sistema.hash().to_owned(),
                estado_final: estado,
            });
        }
        aplicar(&mut estado, &mut transiciones, Event::FallarGate)?;
        fallo_anterior = primer_fallo;
    }
}
