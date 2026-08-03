//! Prompts de sistema versionados y construcción del prompt de cada intento.
//!
//! El prompt de sistema vive como archivo en el repo del proyecto; su hash
//! SHA-256 se registra en el trace para poder comparar efectividad entre
//! versiones. El prompt de intento se construye SOLO con la spec y el
//! [`ResumenFallo`] estructurado: la firma de [`construir_prompt`] no admite
//! transcripciones.

use crate::AgentError;
use crate::estrategia::Estrategia;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::path::Path;

/// Máximo de caracteres del extracto de salida de un gate que entra al
/// prompt del siguiente intento.
pub const MAX_EXTRACTO: usize = 2000;

/// Resumen estructurado de un fallo de gates: lo único del intento anterior
/// que ve el intento siguiente.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumenFallo {
    /// El gate requerido que falló.
    pub gate: String,
    pub exit_code: Option<i32>,
    /// Cola de la salida del gate, truncada a [`MAX_EXTRACTO`] caracteres.
    pub extracto: String,
}

impl ResumenFallo {
    /// Construye el resumen truncando la salida: se conserva la cola, que en
    /// tests y linters suele traer el error concreto.
    pub fn nuevo(gate: impl Into<String>, exit_code: Option<i32>, salida: &str) -> Self {
        let extracto = if salida.chars().count() > MAX_EXTRACTO {
            let cola: String = salida
                .chars()
                .skip(salida.chars().count() - MAX_EXTRACTO)
                .collect();
            format!("[...truncado...]{cola}")
        } else {
            salida.to_owned()
        };
        Self {
            gate: gate.into(),
            exit_code,
            extracto,
        }
    }
}

/// Un prompt de sistema con su hash SHA-256, para anclarlo en el trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptVersionado {
    texto: String,
    hash: String,
}

impl PromptVersionado {
    pub fn desde_texto(texto: String) -> Self {
        let digest = Sha256::digest(texto.as_bytes());
        let mut hash = String::with_capacity(64);
        for byte in digest {
            let _ = write!(hash, "{byte:02x}");
        }
        Self { texto, hash }
    }

    /// Carga el prompt desde un archivo del repo.
    pub fn cargar(path: &Path) -> Result<Self, AgentError> {
        Ok(Self::desde_texto(std::fs::read_to_string(path)?))
    }

    pub fn texto(&self) -> &str {
        &self.texto
    }

    pub fn hash(&self) -> &str {
        &self.hash
    }
}

/// Contexto fresco de un intento.
#[derive(Debug, Clone, Copy)]
pub struct ContextoIntento<'a> {
    pub spec: &'a str,
    pub numero: u32,
    pub estrategia: Estrategia,
    /// Resumen del fallo del intento anterior; `None` en el primer intento.
    pub fallo_anterior: Option<&'a ResumenFallo>,
}

/// Construye el prompt del intento según la estrategia.
pub fn construir_prompt(ctx: ContextoIntento<'_>) -> String {
    let mut prompt = format!(
        "## Requerimiento (intento {} de {})\n\n{}\n",
        ctx.numero,
        crate::estrategia::MAX_INTENTOS,
        ctx.spec
    );
    if let Some(fallo) = ctx.fallo_anterior {
        let _ = write!(
            prompt,
            "\n## Fallo del intento anterior\n\nGate `{}` (exit code {}):\n```\n{}\n```\n",
            fallo.gate,
            fallo
                .exit_code
                .map_or_else(|| "desconocido".to_owned(), |c| c.to_string()),
            fallo.extracto
        );
    }
    let instruccion = match ctx.estrategia {
        Estrategia::Directo => "Implementa el requerimiento.",
        Estrategia::ConFeedback => {
            "Implementa el requerimiento corrigiendo la causa del fallo anterior."
        }
        Estrategia::ContextoLimpioConHipotesis => {
            "Los intentos anteriores fallaron. Antes de escribir código, formula \
             una hipótesis concreta de por qué falla y después implementa según \
             esa hipótesis."
        }
        Estrategia::DiagnosticoPrevio => {
            "Último intento. NO escribas código todavía: primero diagnostica el \
             problema (lee el código y el fallo, reproduce mentalmente la ruta). \
             Solo cuando tengas la causa raíz identificada, aplica el arreglo \
             mínimo."
        }
    };
    let _ = write!(prompt, "\n## Instrucción\n\n{instruccion}\n");
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_hash_es_estable_y_cambia_con_el_texto() {
        let a = PromptVersionado::desde_texto("hola".to_owned());
        let b = PromptVersionado::desde_texto("hola".to_owned());
        let c = PromptVersionado::desde_texto("adios".to_owned());
        assert_eq!(a.hash(), b.hash());
        assert_ne!(a.hash(), c.hash());
        assert_eq!(a.hash().len(), 64);
    }

    #[test]
    fn cargar_desde_archivo_conserva_el_texto() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("sistema.md");
        std::fs::write(&ruta, "eres un agente").unwrap();
        let prompt = PromptVersionado::cargar(&ruta).unwrap();
        assert_eq!(prompt.texto(), "eres un agente");
        assert_eq!(
            prompt.hash(),
            PromptVersionado::desde_texto("eres un agente".to_owned()).hash()
        );
    }

    #[test]
    fn resumen_de_fallo_trunca_conservando_la_cola() {
        let salida = format!("{}ERROR FINAL", "x".repeat(MAX_EXTRACTO * 2));
        let resumen = ResumenFallo::nuevo("tests", Some(1), &salida);
        assert!(resumen.extracto.starts_with("[...truncado...]"));
        assert!(resumen.extracto.ends_with("ERROR FINAL"));
        assert!(resumen.extracto.chars().count() <= MAX_EXTRACTO + 20);
    }

    #[test]
    fn el_primer_intento_no_lleva_feedback() {
        let prompt = construir_prompt(ContextoIntento {
            spec: "sumar dos números",
            numero: 1,
            estrategia: Estrategia::Directo,
            fallo_anterior: None,
        });
        assert!(prompt.contains("sumar dos números"));
        assert!(!prompt.contains("Fallo del intento anterior"));
    }

    #[test]
    fn con_feedback_incluye_el_resumen_del_fallo() {
        let fallo = ResumenFallo::nuevo("tests", Some(2), "assert_eq fallo en linea 42");
        let prompt = construir_prompt(ContextoIntento {
            spec: "sumar dos números",
            numero: 2,
            estrategia: Estrategia::ConFeedback,
            fallo_anterior: Some(&fallo),
        });
        assert!(prompt.contains("Gate `tests` (exit code 2)"));
        assert!(prompt.contains("assert_eq fallo en linea 42"));
    }

    #[test]
    fn el_ultimo_intento_pide_diagnostico_antes_de_codigo() {
        let prompt = construir_prompt(ContextoIntento {
            spec: "spec",
            numero: 5,
            estrategia: Estrategia::DiagnosticoPrevio,
            fallo_anterior: None,
        });
        assert!(prompt.contains("NO escribas código todavía"));
    }
}
