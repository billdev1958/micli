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

/// Tope de stderr que se conserva en [`AgentError::AgenteNoArranco`].
const MAX_STDERR: usize = 2000;

/// Lo que dejó el proceso del agente al terminar, antes de interpretarlo.
struct SalidaProceso<'a> {
    exit_ok: bool,
    exit_code: Option<i32>,
    stdout: &'a str,
    stderr: &'a str,
    duracion: Duration,
}

/// Decide si el proceso fue un intento o un fallo de arranque.
///
/// - stdout con JSON de resultado → intento (con sus tokens, aunque el
///   proceso haya salido con error: Claude Code emite `is_error: true` y
///   código distinto de cero cuando falla ya corriendo).
/// - sin JSON y código de salida distinto de cero → el agente no arrancó;
///   se devuelve [`AgentError::AgenteNoArranco`] con la cola de stderr.
/// - sin JSON y salida limpia → respuesta ilegible.
fn interpretar_salida(salida: SalidaProceso<'_>) -> Result<ResultadoIntento, AgentError> {
    match parsear_respuesta(salida.stdout, salida.duracion) {
        Ok(resultado) => Ok(ResultadoIntento {
            exito: resultado.exito && salida.exit_ok,
            ..resultado
        }),
        Err(_) if !salida.exit_ok => Err(AgentError::AgenteNoArranco {
            exit_code: salida.exit_code,
            stderr: cola(salida.stderr.trim(), MAX_STDERR),
        }),
        Err(error) => Err(error),
    }
}

/// Últimos `max` caracteres de `texto` (lo más informativo de un stderr).
fn cola(texto: &str, max: usize) -> String {
    let total = texto.chars().count();
    if total <= max {
        return texto.to_owned();
    }
    texto.chars().skip(total - max).collect()
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
        interpretar_salida(SalidaProceso {
            exit_ok: salida.status.success(),
            exit_code: salida.status.code(),
            stdout: &String::from_utf8_lossy(&salida.stdout),
            stderr: &String::from_utf8_lossy(&salida.stderr),
            duracion,
        })
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

    fn salida<'a>(exit_code: i32, stdout: &'a str, stderr: &'a str) -> SalidaProceso<'a> {
        SalidaProceso {
            exit_ok: exit_code == 0,
            exit_code: Some(exit_code),
            stdout,
            stderr,
            duracion: Duration::from_secs(3),
        }
    }

    #[test]
    fn exit_distinto_de_cero_sin_json_es_fallo_de_arranque_con_stderr() {
        let error =
            interpretar_salida(salida(1, "", "Not logged in. Run `claude login`.\n")).unwrap_err();
        match error {
            AgentError::AgenteNoArranco { exit_code, stderr } => {
                assert_eq!(exit_code, Some(1));
                assert_eq!(stderr, "Not logged in. Run `claude login`.");
            }
            otro => panic!("se esperaba AgenteNoArranco, llegó {otro:?}"),
        }
    }

    #[test]
    fn exit_distinto_de_cero_con_json_es_intento_fallido_y_cuenta_tokens() {
        // Claude Code sale con 1 e `is_error: true` cuando falla ya corriendo:
        // eso sí es un intento y sus tokens deben contarse.
        let json =
            r#"{"type":"result","is_error":true,"usage":{"input_tokens":700,"output_tokens":50}}"#;
        let resultado = interpretar_salida(salida(1, json, "error de API")).unwrap();
        assert!(!resultado.exito);
        assert_eq!(resultado.tokens, 750);
        assert_eq!(resultado.duracion, Duration::from_secs(3));
    }

    #[test]
    fn exit_cero_con_json_ilegible_sigue_siendo_respuesta_ilegible() {
        let error = interpretar_salida(salida(0, "no json", "")).unwrap_err();
        assert!(matches!(error, AgentError::Json(_)), "{error:?}");
    }

    #[test]
    fn exit_distinto_de_cero_con_json_sin_error_no_cuenta_como_exito() {
        let resultado = interpretar_salida(salida(2, r#"{"is_error": false}"#, "")).unwrap();
        assert!(!resultado.exito);
    }

    #[test]
    fn el_stderr_se_trunca_conservando_la_cola() {
        let largo: String = "x"
            .repeat(MAX_STDERR)
            .chars()
            .chain("FIN".chars())
            .collect();
        let error = interpretar_salida(salida(1, "", &largo)).unwrap_err();
        match error {
            AgentError::AgenteNoArranco { stderr, .. } => {
                assert_eq!(stderr.chars().count(), MAX_STDERR);
                assert!(stderr.ends_with("FIN"));
            }
            otro => panic!("se esperaba AgenteNoArranco, llegó {otro:?}"),
        }
        // Sin exceso no se toca.
        assert_eq!(cola("corto", 10), "corto");
    }
}
