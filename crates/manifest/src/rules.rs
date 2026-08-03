//! Modelo de `agent_rules.toml`: las reglas declarativas que micli impone a
//! un proyecto objetivo. micli es agnóstico al lenguaje del proyecto; estas
//! reglas son configurables por repo, con perfiles predefinidos.

use crate::ManifestError;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Perfiles predefinidos para proyectos objetivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Perfil {
    Rust,
    Angular,
    PythonFastapi,
    Node,
    #[serde(alias = "golang")]
    Go,
}

/// Sección `[project]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectInfo {
    pub name: String,
    pub profile: Perfil,
}

/// Una compuerta de calidad: un comando determinista cuyo código de salida
/// decide. El LLM nunca es compuerta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gate {
    pub name: String,
    pub cmd: String,
    #[serde(default = "verdadero")]
    pub required: bool,
}

fn verdadero() -> bool {
    true
}

/// Sección `[protected]`: rutas que el agente no puede tocar y archivos de
/// dependencias cuya edición requiere aprobación.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Protected {
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub dependency_files: Vec<String>,
}

/// Red disponible dentro del sandbox.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SandboxNetwork {
    #[default]
    None,
    Bridge,
}

/// Sección `[sandbox]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxConfig {
    pub image: String,
    #[serde(default)]
    pub network: SandboxNetwork,
}

/// Sección `[budget]`: presupuestos duros impuestos por micli en Rust.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    pub max_attempts: u32,
    pub max_minutes: u64,
}

/// El archivo `agent_rules.toml` completo de un proyecto objetivo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRules {
    /// Versión del esquema, para migraciones futuras.
    pub version: u32,
    pub project: ProjectInfo,
    #[serde(default)]
    pub gates: Vec<Gate>,
    #[serde(default)]
    pub protected: Protected,
    #[serde(default)]
    pub sandbox: Option<SandboxConfig>,
    #[serde(default)]
    pub budget: Option<Budget>,
}

impl AgentRules {
    /// Versión actual del esquema.
    pub const VERSION: u32 = 1;

    /// Parsea y valida el contenido de un `agent_rules.toml`.
    pub fn parse(contenido: &str) -> Result<Self, ManifestError> {
        let reglas: Self = toml::from_str(contenido)?;
        reglas.validar()?;
        Ok(reglas)
    }

    /// Lee, parsea y valida el archivo en `path`.
    pub fn cargar(path: &Path) -> Result<Self, ManifestError> {
        let contenido = std::fs::read_to_string(path)?;
        Self::parse(&contenido)
    }

    fn validar(&self) -> Result<(), ManifestError> {
        if self.version != Self::VERSION {
            return Err(ManifestError::VersionNoSoportada {
                encontrada: self.version,
                soportada: Self::VERSION,
            });
        }
        if self.project.name.trim().is_empty() {
            return Err(ManifestError::NombreProyectoVacio);
        }
        let mut vistos = std::collections::HashSet::new();
        for gate in &self.gates {
            if gate.name.trim().is_empty() {
                return Err(ManifestError::GateInvalido {
                    gate: gate.name.clone(),
                    motivo: "el nombre está vacío".to_owned(),
                });
            }
            if gate.cmd.trim().is_empty() {
                return Err(ManifestError::GateInvalido {
                    gate: gate.name.clone(),
                    motivo: "el comando está vacío".to_owned(),
                });
            }
            if !vistos.insert(gate.name.as_str()) {
                return Err(ManifestError::GateDuplicado {
                    gate: gate.name.clone(),
                });
            }
        }
        Ok(())
    }

    /// Los gates que deciden la aceptación (los `required`).
    pub fn gates_requeridos(&self) -> impl Iterator<Item = &Gate> {
        self.gates.iter().filter(|g| g.required)
    }

    /// Serializa y escribe el archivo en `path`, creando el directorio padre
    /// si falta. Falla si el archivo ya existe: las reglas de un proyecto se
    /// editan a mano, no se regeneran.
    pub fn guardar(&self, path: &Path) -> Result<(), ManifestError> {
        crate::escribir_nuevo(path, &toml::to_string_pretty(self)?)
    }

    /// Reglas iniciales para un proyecto según su perfil predefinido.
    pub fn plantilla(name: String, profile: Perfil) -> Self {
        let (gates, image) = match profile {
            Perfil::Rust => (
                vec![
                    gate("tests", "cargo test --workspace"),
                    gate(
                        "lint",
                        "cargo clippy --workspace --all-targets -- -D warnings",
                    ),
                ],
                "rust:1-slim",
            ),
            Perfil::Angular => (
                vec![
                    gate("tests", "npm test -- --watch=false"),
                    gate("lint", "npm run lint"),
                ],
                "node:22-slim",
            ),
            Perfil::PythonFastapi => (
                vec![
                    gate("tests", "pytest -x --tb=short"),
                    gate("lint", "ruff check ."),
                ],
                "python:3.12-slim",
            ),
            Perfil::Node => (
                vec![gate("tests", "npm test"), gate("lint", "npm run lint")],
                "node:22-slim",
            ),
            Perfil::Go => (
                vec![gate("tests", "go test ./..."), gate("vet", "go vet ./...")],
                "golang:1-alpine",
            ),
        };
        Self {
            version: Self::VERSION,
            project: ProjectInfo { name, profile },
            gates,
            protected: Protected::default(),
            sandbox: Some(SandboxConfig {
                image: image.to_owned(),
                network: SandboxNetwork::None,
            }),
            budget: Some(Budget {
                max_attempts: 5,
                max_minutes: 30,
            }),
        }
    }
}

fn gate(name: &str, cmd: &str) -> Gate {
    Gate {
        name: name.to_owned(),
        cmd: cmd.to_owned(),
        required: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El ejemplo completo documentado en CLAUDE.md.
    const EJEMPLO: &str = r#"
        version = 1

        [project]
        name = "sync-backend"
        profile = "python-fastapi"

        [[gates]]
        name = "tests"
        cmd = "pytest -x --tb=short"
        required = true

        [[gates]]
        name = "lint"
        cmd = "ruff check ."
        required = true

        [protected]
        paths = ["tests/fixtures/**"]
        dependency_files = ["requirements.txt"]

        [sandbox]
        image = "python:3.12-slim"
        network = "none"

        [budget]
        max_attempts = 5
        max_minutes = 30
    "#;

    #[test]
    fn parsea_el_ejemplo_de_referencia() {
        let reglas = AgentRules::parse(EJEMPLO).unwrap();
        assert_eq!(reglas.project.name, "sync-backend");
        assert_eq!(reglas.project.profile, Perfil::PythonFastapi);
        assert_eq!(reglas.gates.len(), 2);
        assert_eq!(reglas.gates_requeridos().count(), 2);
        assert_eq!(reglas.protected.paths, vec!["tests/fixtures/**"]);
        let sandbox = reglas.sandbox.unwrap();
        assert_eq!(sandbox.image, "python:3.12-slim");
        assert_eq!(sandbox.network, SandboxNetwork::None);
        let budget = reglas.budget.unwrap();
        assert_eq!(budget.max_attempts, 5);
        assert_eq!(budget.max_minutes, 30);
    }

    #[test]
    fn minimo_valido_con_secciones_opcionales_por_defecto() {
        let reglas = AgentRules::parse(
            r#"
            version = 1
            [project]
            name = "demo"
            profile = "rust"
        "#,
        )
        .unwrap();
        assert!(reglas.gates.is_empty());
        assert_eq!(reglas.protected, Protected::default());
        assert!(reglas.sandbox.is_none());
        assert!(reglas.budget.is_none());
    }

    #[test]
    fn required_es_true_por_defecto() {
        let reglas = AgentRules::parse(
            r#"
            version = 1
            [project]
            name = "demo"
            profile = "node"
            [[gates]]
            name = "tests"
            cmd = "npm test"
        "#,
        )
        .unwrap();
        assert!(reglas.gates[0].required);
    }

    #[test]
    fn el_perfil_go_acepta_el_alias_golang() {
        for nombre in ["go", "golang"] {
            let reglas = AgentRules::parse(&format!(
                "version = 1\n[project]\nname = \"demo\"\nprofile = \"{nombre}\"\n"
            ))
            .unwrap();
            assert_eq!(reglas.project.profile, Perfil::Go, "perfil `{nombre}`");
        }
    }

    #[test]
    fn rechaza_version_desconocida() {
        let error = AgentRules::parse(
            r#"
            version = 99
            [project]
            name = "demo"
            profile = "rust"
        "#,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ManifestError::VersionNoSoportada { encontrada: 99, .. }
        ));
    }

    #[test]
    fn rechaza_campos_desconocidos_y_perfil_invalido() {
        assert!(
            AgentRules::parse(
                r#"
                version = 1
                intruso = true
                [project]
                name = "demo"
                profile = "rust"
            "#,
            )
            .is_err()
        );
        assert!(
            AgentRules::parse(
                r#"
                version = 1
                [project]
                name = "demo"
                profile = "cobol"
            "#,
            )
            .is_err()
        );
    }

    #[test]
    fn rechaza_gates_duplicados_o_vacios() {
        let duplicado = AgentRules::parse(
            r#"
            version = 1
            [project]
            name = "demo"
            profile = "rust"
            [[gates]]
            name = "tests"
            cmd = "cargo test"
            [[gates]]
            name = "tests"
            cmd = "cargo test --release"
        "#,
        )
        .unwrap_err();
        assert!(matches!(duplicado, ManifestError::GateDuplicado { .. }));

        let vacio = AgentRules::parse(
            r#"
            version = 1
            [project]
            name = "demo"
            profile = "rust"
            [[gates]]
            name = "tests"
            cmd = "   "
        "#,
        )
        .unwrap_err();
        assert!(matches!(vacio, ManifestError::GateInvalido { .. }));
    }

    #[test]
    fn las_plantillas_de_todos_los_perfiles_son_validas_y_roundtripean() {
        for perfil in [
            Perfil::Rust,
            Perfil::Angular,
            Perfil::PythonFastapi,
            Perfil::Node,
            Perfil::Go,
        ] {
            let plantilla = AgentRules::plantilla("demo".to_owned(), perfil);
            let toml = toml::to_string_pretty(&plantilla).unwrap();
            let releida = AgentRules::parse(&toml).unwrap();
            assert_eq!(releida, plantilla, "perfil {perfil:?}");
            assert!(releida.gates_requeridos().count() >= 2);
            assert_eq!(
                releida.sandbox.unwrap().network,
                SandboxNetwork::None,
                "el sandbox de plantilla nunca abre red"
            );
        }
    }

    #[test]
    fn guardar_no_sobreescribe_reglas_existentes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".micli").join("agent_rules.toml");
        let plantilla = AgentRules::plantilla("demo".to_owned(), Perfil::Rust);
        plantilla.guardar(&path).unwrap();
        assert_eq!(AgentRules::cargar(&path).unwrap(), plantilla);
        assert!(matches!(
            plantilla.guardar(&path),
            Err(ManifestError::ArchivoYaExiste { .. })
        ));
    }

    #[test]
    fn rechaza_nombre_de_proyecto_vacio() {
        let error = AgentRules::parse(
            r#"
            version = 1
            [project]
            name = "  "
            profile = "rust"
        "#,
        )
        .unwrap_err();
        assert!(matches!(error, ManifestError::NombreProyectoVacio));
    }
}
