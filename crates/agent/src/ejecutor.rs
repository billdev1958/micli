//! Invocación del agente de código. [`Ejecutor`] abstrae al proveedor;
//! [`ClaudeCodeCli`] es la implementación real sobre `claude -p` en modo no
//! interactivo con salida JSON.

use crate::AgentError;
use crate::prompt::PromptVersionado;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Lo que se le pide a un intento.
#[derive(Debug, Clone)]
pub struct SolicitudIntento<'a> {
    pub prompt_sistema: &'a PromptVersionado,
    pub prompt: String,
    pub dir_trabajo: &'a Path,
}

/// Lo que reporta un intento terminado.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResultadoIntento {
    /// El agente terminó sin error propio (no dice nada de los gates).
    pub exito: bool,
    pub tokens: u64,
    pub duracion: Duration,
}

/// Quién corre un intento. Se inyecta para poder testear el bucle sin
/// procesos reales.
pub trait Ejecutor {
    fn ejecutar(&mut self, solicitud: SolicitudIntento<'_>)
    -> Result<ResultadoIntento, AgentError>;
}

/// Claude Code en modo no interactivo: `claude -p <prompt> --output-format
/// json --append-system-prompt <sistema>`, en el directorio del proyecto.
///
/// El presupuesto de tiempo se impone entre intentos (el proceso no se mata
/// a media ejecución); el tope duro por proceso llegará con el sandbox
/// (Fase 5).
#[derive(Debug, Clone)]
pub struct ClaudeCodeCli {
    pub binario: PathBuf,
    /// Flags extra (p. ej. `--permission-mode`), configurables por micli.
    pub args_extra: Vec<String>,
}

impl Default for ClaudeCodeCli {
    fn default() -> Self {
        Self {
            binario: PathBuf::from("claude"),
            args_extra: Vec::new(),
        }
    }
}

impl ClaudeCodeCli {
    /// Argumentos del proceso (separados para poder testearlos sin ejecutar).
    fn argumentos(&self, solicitud: &SolicitudIntento<'_>) -> Vec<String> {
        let mut args = vec![
            "-p".to_owned(),
            solicitud.prompt.clone(),
            "--output-format".to_owned(),
            "json".to_owned(),
            "--append-system-prompt".to_owned(),
            solicitud.prompt_sistema.texto().to_owned(),
        ];
        args.extend(self.args_extra.iter().cloned());
        args
    }
}

/// Interpreta la respuesta JSON de `claude -p --output-format json`.
/// Tolerante a campos ausentes: si no hay `usage`, los tokens quedan en 0.
fn parsear_respuesta(json: &str, duracion: Duration) -> Result<ResultadoIntento, AgentError> {
    let valor: serde_json::Value = serde_json::from_str(json)?;
    let exito = !valor
        .get("is_error")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let uso = valor.get("usage");
    let tokens = uso
        .and_then(|u| u.get("input_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
        .saturating_add(
            uso.and_then(|u| u.get("output_tokens"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        );
    Ok(ResultadoIntento {
        exito,
        tokens,
        duracion,
    })
}

impl Ejecutor for ClaudeCodeCli {
    fn ejecutar(
        &mut self,
        solicitud: SolicitudIntento<'_>,
    ) -> Result<ResultadoIntento, AgentError> {
        let inicio = Instant::now();
        let salida = std::process::Command::new(&self.binario)
            .args(self.argumentos(&solicitud))
            .current_dir(solicitud.dir_trabajo)
            .output()
            .map_err(|fuente| AgentError::AgenteNoEjecutable {
                binario: self.binario.display().to_string(),
                fuente,
            })?;
        let duracion = inicio.elapsed();
        if !salida.status.success() {
            return Ok(ResultadoIntento {
                exito: false,
                tokens: 0,
                duracion,
            });
        }
        parsear_respuesta(&String::from_utf8_lossy(&salida.stdout), duracion)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arma_los_argumentos_de_claude_p_con_json() {
        let sistema = PromptVersionado::desde_texto("sistema".to_owned());
        let cli = ClaudeCodeCli {
            binario: PathBuf::from("claude"),
            args_extra: vec!["--permission-mode".to_owned(), "acceptEdits".to_owned()],
        };
        let dir = PathBuf::from(".");
        let args = cli.argumentos(&SolicitudIntento {
            prompt_sistema: &sistema,
            prompt: "haz algo".to_owned(),
            dir_trabajo: &dir,
        });
        assert_eq!(
            args,
            vec![
                "-p",
                "haz algo",
                "--output-format",
                "json",
                "--append-system-prompt",
                "sistema",
                "--permission-mode",
                "acceptEdits",
            ]
        );
    }

    #[test]
    fn parsea_la_respuesta_json_de_claude() {
        let json = r#"{
            "type": "result",
            "is_error": false,
            "result": "listo",
            "usage": { "input_tokens": 1200, "output_tokens": 345 }
        }"#;
        let resultado = parsear_respuesta(json, Duration::from_secs(9)).unwrap();
        assert!(resultado.exito);
        assert_eq!(resultado.tokens, 1545);
        assert_eq!(resultado.duracion, Duration::from_secs(9));
    }

    #[test]
    fn respuesta_con_error_o_sin_usage_es_tolerada() {
        let con_error = parsear_respuesta(r#"{"is_error": true}"#, Duration::ZERO).unwrap();
        assert!(!con_error.exito);
        assert_eq!(con_error.tokens, 0);

        let minima = parsear_respuesta("{}", Duration::ZERO).unwrap();
        assert!(minima.exito);
        assert_eq!(minima.tokens, 0);

        assert!(parsear_respuesta("esto no es json", Duration::ZERO).is_err());
    }
}
