//! Ejecución aislada en Docker vía `bollard`, con imagen parametrizada por
//! proyecto.
//!
//! Regla dura #4: el contenedor NUNCA recibe credenciales, variables de
//! entorno del host ni el socket de Docker. El mecanismo es
//! [`cuerpo_contenedor`]: una función pura que genera la especificación
//! completa del contenedor, inspeccionada por tests. El único acceso al
//! host es el bind del directorio del proyecto en `/workspace`.
//!
//! El timeout es duro: al agotarse se mata el contenedor.

use bollard::Docker;
use bollard::models::{ContainerCreateBody, HostConfig};
use bollard::query_parameters::{
    CreateContainerOptions, KillContainerOptions, LogsOptionsBuilder,
    RemoveContainerOptionsBuilder, StartContainerOptions, WaitContainerOptions,
};
use futures_util::StreamExt;
use std::path::PathBuf;
use std::time::Duration;

/// Punto de montaje del proyecto dentro del contenedor.
pub const WORKSPACE: &str = "/workspace";

/// Errores del sandbox.
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error(
        "no se pudo hablar con Docker: {0}. ¿Está corriendo Docker y existe la imagen? (docker pull <imagen>)"
    )]
    Docker(#[from] bollard::errors::Error),
    #[error("no se pudo crear el runtime de ejecución: {0}")]
    Runtime(#[from] std::io::Error),
}

/// Red disponible dentro del contenedor. Sin red por defecto.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Red {
    #[default]
    None,
    Bridge,
}

impl Red {
    fn network_mode(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Bridge => "bridge",
        }
    }
}

/// Configuración del sandbox de un proyecto.
#[derive(Debug, Clone)]
pub struct ConfigSandbox {
    pub imagen: String,
    pub red: Red,
    pub dir_proyecto: PathBuf,
    /// Tope duro por ejecución: al agotarse se mata el contenedor.
    pub timeout: Duration,
}

/// Resultado de una ejecución dentro del sandbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EjecucionSandbox {
    /// `None` si el contenedor fue matado por timeout.
    pub exit_code: Option<i64>,
    /// stdout + stderr del contenedor.
    pub salida: String,
    pub timeout_agotado: bool,
}

impl EjecucionSandbox {
    pub fn paso(&self) -> bool {
        !self.timeout_agotado && self.exit_code == Some(0)
    }
}

/// Genera la especificación completa del contenedor. Función pura: los tests
/// de la regla dura #4 inspeccionan exactamente lo que Docker recibirá.
///
/// Garantías:
/// - `env` vacío explícito: no se hereda NINGUNA variable del host.
/// - Un único bind: el directorio del proyecto en [`WORKSPACE`]. Jamás el
///   socket de Docker ni rutas de credenciales.
/// - Sin privilegios y con todas las capabilities de Linux retiradas.
/// - Red según configuración (`none` por defecto).
pub fn cuerpo_contenedor(config: &ConfigSandbox, cmd: &str) -> ContainerCreateBody {
    ContainerCreateBody {
        image: Some(config.imagen.clone()),
        cmd: Some(vec!["sh".to_owned(), "-c".to_owned(), cmd.to_owned()]),
        working_dir: Some(WORKSPACE.to_owned()),
        env: Some(Vec::new()),
        host_config: Some(HostConfig {
            binds: Some(vec![format!(
                "{}:{WORKSPACE}",
                ruta_bind(&config.dir_proyecto)
            )]),
            network_mode: Some(config.red.network_mode().to_owned()),
            privileged: Some(false),
            cap_drop: Some(vec!["ALL".to_owned()]),
            ..HostConfig::default()
        }),
        ..ContainerCreateBody::default()
    }
}

/// Ruta del proyecto en el formato que el daemon de Docker acepta como bind.
/// `Path::canonicalize` en Windows produce el prefijo verbatim `\\?\` (p. ej.
/// `\\?\C:\dev\proyecto`); Docker parte el bind por `:` y rechaza `\\?\C` como
/// "not a valid Windows path", así que el prefijo se quita aquí.
fn ruta_bind(dir: &std::path::Path) -> String {
    let ruta = dir.display().to_string();
    match ruta.strip_prefix(r"\\?\") {
        Some(sin_prefijo) => sin_prefijo.to_owned(),
        None => ruta,
    }
}

/// Conexión al daemon local de Docker para un proyecto.
#[derive(Debug)]
pub struct Sandbox {
    docker: Docker,
    config: ConfigSandbox,
}

impl Sandbox {
    /// Conecta con el Docker local (named pipe en Windows, socket en Unix).
    pub fn nuevo(config: ConfigSandbox) -> Result<Self, SandboxError> {
        Ok(Self {
            docker: Docker::connect_with_local_defaults()?,
            config,
        })
    }

    /// Corre `cmd` dentro del contenedor y espera a que termine (o mata el
    /// contenedor si se agota el timeout). API síncrona: crea su runtime.
    pub fn ejecutar(&self, cmd: &str) -> Result<EjecucionSandbox, SandboxError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(self.ejecutar_async(cmd))
    }

    async fn ejecutar_async(&self, cmd: &str) -> Result<EjecucionSandbox, SandboxError> {
        let creado = self
            .docker
            .create_container(
                None::<CreateContainerOptions>,
                cuerpo_contenedor(&self.config, cmd),
            )
            .await?;
        let id = creado.id;
        let resultado = self.correr(&id).await;
        // El contenedor se elimina siempre, incluso si algo falló a medias.
        let _ = self
            .docker
            .remove_container(
                &id,
                Some(RemoveContainerOptionsBuilder::new().force(true).build()),
            )
            .await;
        resultado
    }

    async fn correr(&self, id: &str) -> Result<EjecucionSandbox, SandboxError> {
        self.docker
            .start_container(id, None::<StartContainerOptions>)
            .await?;

        let mut espera =
            std::pin::pin!(self.docker.wait_container(id, None::<WaitContainerOptions>));
        let (exit_code, timeout_agotado) =
            match tokio::time::timeout(self.config.timeout, espera.next()).await {
                // bollard reporta exit != 0 como error tipado del wait.
                Ok(Some(Ok(r))) => (Some(r.status_code), false),
                Ok(Some(Err(bollard::errors::Error::DockerContainerWaitError {
                    code, ..
                }))) => (Some(code), false),
                Ok(Some(Err(e))) => return Err(e.into()),
                Ok(None) => (None, false),
                Err(_) => {
                    let _ = self
                        .docker
                        .kill_container(id, None::<KillContainerOptions>)
                        .await;
                    (None, true)
                }
            };

        let mut logs = self.docker.logs(
            id,
            Some(LogsOptionsBuilder::new().stdout(true).stderr(true).build()),
        );
        let mut salida = String::new();
        while let Some(fragmento) = logs.next().await {
            match fragmento {
                Ok(log) => salida.push_str(&log.to_string()),
                Err(_) => break,
            }
        }

        Ok(EjecucionSandbox {
            exit_code,
            salida,
            timeout_agotado,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(red: Red) -> ConfigSandbox {
        ConfigSandbox {
            imagen: "python:3.12-slim".to_owned(),
            red,
            dir_proyecto: PathBuf::from("C:/dev/proyecto"),
            timeout: Duration::from_secs(60),
        }
    }

    /// Regla dura #4: inspección de la especificación completa que recibirá
    /// Docker. Estos tests son el mecanismo de la regla, no documentación.
    #[test]
    fn el_contenedor_no_hereda_ninguna_variable_de_entorno() {
        let cuerpo = cuerpo_contenedor(&config(Red::None), "pytest");
        assert_eq!(cuerpo.env, Some(Vec::new()));
    }

    #[test]
    fn el_unico_bind_es_el_proyecto_y_nunca_el_socket_de_docker() {
        let cuerpo = cuerpo_contenedor(&config(Red::None), "pytest");
        let host = cuerpo.host_config.unwrap();
        let binds = host.binds.unwrap();
        assert_eq!(binds, vec!["C:/dev/proyecto:/workspace".to_owned()]);
        for bind in &binds {
            let bind = bind.to_lowercase();
            assert!(!bind.contains("docker.sock"), "monta el socket: {bind}");
            assert!(!bind.contains("docker_engine"), "monta el pipe: {bind}");
            assert!(!bind.contains(".ssh"), "monta credenciales: {bind}");
            assert!(!bind.contains(".aws"), "monta credenciales: {bind}");
        }
        assert!(host.mounts.is_none(), "no debe haber mounts adicionales");
    }

    #[test]
    fn el_bind_quita_el_prefijo_verbatim_de_windows() {
        // canonicalize() en Windows devuelve `\\?\C:\...`; Docker rechaza ese
        // prefijo en un bind ("\\?\C is not a valid Windows path").
        let mut cfg = config(Red::None);
        cfg.dir_proyecto = PathBuf::from(r"\\?\C:\dev\proyecto");
        let cuerpo = cuerpo_contenedor(&cfg, "pytest");
        let binds = cuerpo.host_config.unwrap().binds.unwrap();
        assert_eq!(binds, vec![r"C:\dev\proyecto:/workspace".to_owned()]);
    }

    #[test]
    fn sin_privilegios_sin_capabilities_y_con_la_red_configurada() {
        let cuerpo = cuerpo_contenedor(&config(Red::None), "pytest");
        let host = cuerpo.host_config.unwrap();
        assert_eq!(host.privileged, Some(false));
        assert_eq!(host.cap_drop, Some(vec!["ALL".to_owned()]));
        assert_eq!(host.network_mode.as_deref(), Some("none"));

        let puente = cuerpo_contenedor(&config(Red::Bridge), "pytest");
        assert_eq!(
            puente.host_config.unwrap().network_mode.as_deref(),
            Some("bridge")
        );
    }

    #[test]
    fn el_comando_corre_en_el_workspace_con_la_imagen_del_proyecto() {
        let cuerpo = cuerpo_contenedor(&config(Red::None), "pytest -x");
        assert_eq!(cuerpo.image.as_deref(), Some("python:3.12-slim"));
        assert_eq!(cuerpo.working_dir.as_deref(), Some(WORKSPACE));
        assert_eq!(
            cuerpo.cmd,
            Some(vec![
                "sh".to_owned(),
                "-c".to_owned(),
                "pytest -x".to_owned()
            ])
        );
    }
}
