//! Modelo de `requirements.toml`: la lista de requerimientos declarados de
//! un proyecto. El estado de cada requerimiento NO vive aquí sino en el
//! trace; este archivo solo declara qué requerimientos existen.

use crate::ManifestError;
use micli_core::RequirementId;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Un requerimiento declarado.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequirementSpec {
    pub id: RequirementId,
    pub titulo: String,
    /// Especificación de lo que debe cumplirse; puede quedar vacía en Draft.
    #[serde(default)]
    pub spec: String,
    /// Módulo funcional al que pertenece, para agrupar el avance. Vacío =
    /// sin módulo (se agrupa como "general").
    #[serde(default)]
    pub modulo: String,
}

/// El archivo `requirements.toml` completo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequirementsFile {
    /// Versión del esquema, para migraciones futuras.
    pub version: u32,
    #[serde(default)]
    pub requirements: Vec<RequirementSpec>,
}

impl RequirementsFile {
    /// Versión actual del esquema.
    pub const VERSION: u32 = 1;

    /// Parsea y valida el contenido de un `requirements.toml`.
    pub fn parse(contenido: &str) -> Result<Self, ManifestError> {
        let archivo: Self = toml::from_str(contenido)?;
        archivo.validar()?;
        Ok(archivo)
    }

    /// Lee, parsea y valida el archivo en `path`.
    pub fn cargar(path: &Path) -> Result<Self, ManifestError> {
        let contenido = std::fs::read_to_string(path)?;
        Self::parse(&contenido)
    }

    /// Archivo inicial sin requerimientos declarados.
    pub fn vacio() -> Self {
        Self {
            version: Self::VERSION,
            requirements: Vec::new(),
        }
    }

    /// Serializa y escribe el archivo en `path`; falla si ya existe.
    pub fn guardar(&self, path: &Path) -> Result<(), ManifestError> {
        crate::escribir_nuevo(path, &toml::to_string_pretty(self)?)
    }

    fn validar(&self) -> Result<(), ManifestError> {
        if self.version != Self::VERSION {
            return Err(ManifestError::VersionNoSoportada {
                encontrada: self.version,
                soportada: Self::VERSION,
            });
        }
        let mut vistos = std::collections::HashSet::new();
        for req in &self.requirements {
            if req.id.as_str().trim().is_empty() {
                return Err(ManifestError::RequirementInvalido {
                    id: req.id.to_string(),
                    motivo: "el id está vacío".to_owned(),
                });
            }
            if req.titulo.trim().is_empty() {
                return Err(ManifestError::RequirementInvalido {
                    id: req.id.to_string(),
                    motivo: "el título está vacío".to_owned(),
                });
            }
            if !vistos.insert(req.id.as_str()) {
                return Err(ManifestError::RequirementDuplicado {
                    id: req.id.to_string(),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsea_una_lista_de_requerimientos() {
        let archivo = RequirementsFile::parse(
            r#"
            version = 1

            [[requirements]]
            id = "REQ-001"
            titulo = "Alta de usuarios"
            spec = "Dado un correo valido..."

            [[requirements]]
            id = "REQ-002"
            titulo = "Login"
            modulo = "auth"
        "#,
        )
        .unwrap();
        assert_eq!(archivo.requirements.len(), 2);
        assert_eq!(archivo.requirements[0].id.as_str(), "REQ-001");
        assert_eq!(archivo.requirements[0].modulo, "");
        assert_eq!(archivo.requirements[1].spec, "");
        assert_eq!(archivo.requirements[1].modulo, "auth");
    }

    #[test]
    fn archivo_sin_requerimientos_es_valido() {
        let archivo = RequirementsFile::parse("version = 1").unwrap();
        assert!(archivo.requirements.is_empty());
    }

    #[test]
    fn rechaza_ids_duplicados() {
        let error = RequirementsFile::parse(
            r#"
            version = 1
            [[requirements]]
            id = "REQ-001"
            titulo = "A"
            [[requirements]]
            id = "REQ-001"
            titulo = "B"
        "#,
        )
        .unwrap_err();
        assert!(matches!(error, ManifestError::RequirementDuplicado { .. }));
    }

    #[test]
    fn rechaza_titulo_vacio() {
        let error = RequirementsFile::parse(
            r#"
            version = 1
            [[requirements]]
            id = "REQ-001"
            titulo = ""
        "#,
        )
        .unwrap_err();
        assert!(matches!(error, ManifestError::RequirementInvalido { .. }));
    }
}
