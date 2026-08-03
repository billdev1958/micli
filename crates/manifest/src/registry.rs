//! Registro global de proyectos: qué desarrollos están dados de alta en esta
//! instalación de micli. Vive en el ámbito global (`%APPDATA%\micli\` /
//! `~/.micli/`); dónde exactamente lo decide el binario, no este crate.

use crate::ManifestError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Un proyecto registrado.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisteredProject {
    pub name: String,
    pub path: PathBuf,
}

/// El registro global de proyectos (`projects.toml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    /// Versión del esquema, para migraciones futuras.
    pub version: u32,
    #[serde(default)]
    pub projects: Vec<RegisteredProject>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            version: Self::VERSION,
            projects: Vec::new(),
        }
    }
}

impl Registry {
    /// Versión actual del esquema.
    pub const VERSION: u32 = 1;

    /// Carga el registro desde `path`; si el archivo no existe devuelve un
    /// registro vacío (primera ejecución).
    pub fn cargar(path: &Path) -> Result<Self, ManifestError> {
        let contenido = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.into()),
        };
        let registro: Self = toml::from_str(&contenido)?;
        if registro.version != Self::VERSION {
            return Err(ManifestError::VersionNoSoportada {
                encontrada: registro.version,
                soportada: Self::VERSION,
            });
        }
        Ok(registro)
    }

    /// Persiste el registro en `path`, creando el directorio padre si falta.
    pub fn guardar(&self, path: &Path) -> Result<(), ManifestError> {
        if let Some(padre) = path.parent()
            && !padre.as_os_str().is_empty()
        {
            std::fs::create_dir_all(padre)?;
        }
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Da de alta un proyecto. Rechaza nombres o rutas ya registrados.
    pub fn registrar(&mut self, name: String, path: PathBuf) -> Result<(), ManifestError> {
        if name.trim().is_empty() {
            return Err(ManifestError::NombreProyectoVacio);
        }
        if self.projects.iter().any(|p| p.name == name) {
            return Err(ManifestError::ProyectoDuplicado { proyecto: name });
        }
        if self.projects.iter().any(|p| p.path == path) {
            return Err(ManifestError::ProyectoDuplicado {
                proyecto: path.display().to_string(),
            });
        }
        self.projects.push(RegisteredProject { name, path });
        Ok(())
    }

    /// Busca un proyecto por nombre.
    pub fn buscar(&self, name: &str) -> Option<&RegisteredProject> {
        self.projects.iter().find(|p| p.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargar_sin_archivo_devuelve_registro_vacio() {
        let dir = tempfile::tempdir().unwrap();
        let registro = Registry::cargar(&dir.path().join("projects.toml")).unwrap();
        assert!(registro.projects.is_empty());
    }

    #[test]
    fn registrar_guardar_y_recargar() {
        let dir = tempfile::tempdir().unwrap();
        let archivo = dir.path().join("micli").join("projects.toml");

        let mut registro = Registry::default();
        registro
            .registrar("sync-backend".to_owned(), PathBuf::from("C:/dev/sync"))
            .unwrap();
        registro.guardar(&archivo).unwrap();

        let recargado = Registry::cargar(&archivo).unwrap();
        assert_eq!(recargado, registro);
        assert_eq!(
            recargado.buscar("sync-backend").unwrap().path,
            PathBuf::from("C:/dev/sync")
        );
        assert!(recargado.buscar("otro").is_none());
    }

    #[test]
    fn rechaza_duplicados_por_nombre_y_por_ruta() {
        let mut registro = Registry::default();
        registro
            .registrar("a".to_owned(), PathBuf::from("C:/dev/a"))
            .unwrap();
        assert!(matches!(
            registro.registrar("a".to_owned(), PathBuf::from("C:/dev/otro")),
            Err(ManifestError::ProyectoDuplicado { .. })
        ));
        assert!(matches!(
            registro.registrar("b".to_owned(), PathBuf::from("C:/dev/a")),
            Err(ManifestError::ProyectoDuplicado { .. })
        ));
    }
}
