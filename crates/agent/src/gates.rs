//! Ejecución de gates: comandos deterministas cuyo código de salida decide.
//! El LLM nunca es compuerta; las reviews de IA son advisory y viajan por
//! otro canal.

use crate::AgentError;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

/// Un gate a ejecutar. `micli` lo construye desde el `agent_rules.toml` del
/// proyecto (este crate no depende de `manifest`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateSpec {
    pub name: String,
    pub cmd: String,
    pub required: bool,
}

/// Resultado de correr un gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResultadoGate {
    pub gate: String,
    pub paso: bool,
    pub exit_code: Option<i32>,
    /// stdout + stderr combinados.
    pub salida: String,
}

/// Quién ejecuta los gates. En Fase 5 habrá una implementación sobre el
/// sandbox de Docker; mientras, [`GatesEnHost`] corre en la máquina local.
pub trait EjecutorGates {
    fn ejecutar(&mut self, gate: &GateSpec, dir: &Path) -> Result<ResultadoGate, AgentError>;
}

/// Ejecuta los gates con el shell del sistema en el directorio del proyecto.
#[derive(Debug, Default)]
pub struct GatesEnHost;

impl EjecutorGates for GatesEnHost {
    fn ejecutar(&mut self, gate: &GateSpec, dir: &Path) -> Result<ResultadoGate, AgentError> {
        let mut comando = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(&gate.cmd);
            c
        } else {
            let mut c = Command::new("sh");
            c.arg("-c").arg(&gate.cmd);
            c
        };
        let salida =
            comando
                .current_dir(dir)
                .output()
                .map_err(|fuente| AgentError::GateNoEjecutable {
                    gate: gate.name.clone(),
                    fuente,
                })?;
        let mut texto = String::from_utf8_lossy(&salida.stdout).into_owned();
        texto.push_str(&String::from_utf8_lossy(&salida.stderr));
        Ok(ResultadoGate {
            gate: gate.name.clone(),
            paso: salida.status.success(),
            exit_code: salida.status.code(),
            salida: texto,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(cmd: &str) -> GateSpec {
        GateSpec {
            name: "demo".to_owned(),
            cmd: cmd.to_owned(),
            required: true,
        }
    }

    #[test]
    fn exit_cero_pasa_y_captura_salida() {
        let dir = tempfile::tempdir().unwrap();
        let resultado = GatesEnHost
            .ejecutar(&gate("echo hola"), dir.path())
            .unwrap();
        assert!(resultado.paso);
        assert_eq!(resultado.exit_code, Some(0));
        assert!(resultado.salida.contains("hola"));
    }

    #[test]
    fn exit_distinto_de_cero_no_pasa() {
        let dir = tempfile::tempdir().unwrap();
        let resultado = GatesEnHost.ejecutar(&gate("exit 3"), dir.path()).unwrap();
        assert!(!resultado.paso);
        assert_eq!(resultado.exit_code, Some(3));
    }
}
