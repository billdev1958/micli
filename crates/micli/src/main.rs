//! Binario CLI de `micli`: parsing de args, wiring de los crates y salida a
//! terminal. Único crate del workspace con permiso de imprimir.
#![allow(clippy::print_stdout)]

mod avance;
mod comandos;
mod ruflo;
mod rutas;

use clap::{Parser, Subcommand, ValueEnum};
use micli_manifest::Perfil;

#[derive(Parser)]
#[command(
    name = "micli",
    version,
    about = "Arnés de orquestación para agentes de código"
)]
struct Cli {
    #[command(subcommand)]
    comando: Comando,
}

#[derive(Subcommand)]
enum Comando {
    /// Registra el proyecto del directorio actual y genera sus manifests
    Init {
        /// Perfil del proyecto objetivo
        #[arg(long, value_enum)]
        profile: PerfilArg,
        /// Nombre del proyecto (por defecto, el nombre del directorio)
        #[arg(long)]
        name: Option<String>,
    },
    /// Muestra el estado de los requerimientos del proyecto actual
    Status,
    /// Avance de todos los proyectos registrados en esta instalación
    Dashboard,
    /// Ciclo de vida de un requerimiento (transiciones humanas)
    Req {
        #[command(subcommand)]
        comando: ComandoReq,
    },
    /// Ejecuta el bucle del agente para un requerimiento en TestRojo
    Run {
        /// Id del requerimiento (declarado en requirements.toml)
        id: String,
        /// Binario del agente a invocar
        #[arg(long, default_value = "claude")]
        agente: String,
        /// Corre los gates aislados en Docker según [sandbox] de agent_rules.toml
        #[arg(long)]
        sandbox: bool,
        /// Modo de permisos que se pasa a Claude Code (p. ej. acceptEdits)
        #[arg(long)]
        permission_mode: Option<String>,
    },
    /// Integración con ruflo (orquestador MCP para Claude Code)
    Ruflo {
        #[command(subcommand)]
        comando: ComandoRuflo,
    },
    /// Operaciones sobre el log de eventos
    Trace {
        #[command(subcommand)]
        comando: ComandoTrace,
    },
    /// Operaciones sobre la proyección SQLite
    Db {
        #[command(subcommand)]
        comando: ComandoDb,
    },
}

#[derive(Subcommand)]
enum ComandoReq {
    /// Registra en el trace un requerimiento declarado en requirements.toml
    Add { id: String },
    /// Aplica una transición humana al requerimiento
    Avanzar {
        id: String,
        #[arg(long, value_enum)]
        evento: EventoHumano,
    },
}

#[derive(Subcommand)]
enum ComandoTrace {
    /// Verifica la integridad de la cadena de eventos
    Verify,
}

#[derive(Subcommand)]
enum ComandoDb {
    /// Reconstruye la proyección SQLite completa desde el trace
    Rebuild,
}

#[derive(Subcommand)]
enum ComandoRuflo {
    /// Inicializa ruflo en el proyecto (npx ruflo init) y sincroniza reglas
    Init,
    /// Reescribe el bloque de micli en el CLAUDE.md del proyecto desde agent_rules.toml
    Sync,
}

/// Transiciones que dispara un humano; las del agente las dispara `run`.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum EventoHumano {
    AprobarSpec,
    ConfirmarTestRojo,
    Bloquear,
    Desbloquear,
}

impl From<EventoHumano> for micli_core::Event {
    fn from(evento: EventoHumano) -> Self {
        match evento {
            EventoHumano::AprobarSpec => Self::AprobarSpec,
            EventoHumano::ConfirmarTestRojo => Self::ConfirmarTestRojo,
            EventoHumano::Bloquear => Self::Bloquear,
            EventoHumano::Desbloquear => Self::Desbloquear,
        }
    }
}

/// Espejo de `manifest::Perfil` para clap (kebab-case en la terminal).
#[derive(Debug, Clone, Copy, ValueEnum)]
enum PerfilArg {
    Rust,
    Angular,
    PythonFastapi,
    Node,
    #[value(alias = "golang")]
    Go,
}

impl From<PerfilArg> for Perfil {
    fn from(arg: PerfilArg) -> Self {
        match arg {
            PerfilArg::Rust => Self::Rust,
            PerfilArg::Angular => Self::Angular,
            PerfilArg::PythonFastapi => Self::PythonFastapi,
            PerfilArg::Node => Self::Node,
            PerfilArg::Go => Self::Go,
        }
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir()?;
    let salida = match cli.comando {
        Comando::Init { profile, name } => comandos::init(&cwd, name, profile.into())?,
        Comando::Status => comandos::status(&cwd)?,
        Comando::Dashboard => comandos::dashboard()?,
        Comando::Req {
            comando: ComandoReq::Add { id },
        } => comandos::req_add(&cwd, &id)?,
        Comando::Req {
            comando: ComandoReq::Avanzar { id, evento },
        } => comandos::req_avanzar(&cwd, &id, evento.into())?,
        Comando::Run {
            id,
            agente,
            sandbox,
            permission_mode,
        } => comandos::run(&cwd, &id, &agente, sandbox, permission_mode)?,
        Comando::Ruflo {
            comando: ComandoRuflo::Init,
        } => ruflo::init(&cwd)?,
        Comando::Ruflo {
            comando: ComandoRuflo::Sync,
        } => ruflo::sync(&cwd)?,
        Comando::Trace {
            comando: ComandoTrace::Verify,
        } => comandos::trace_verify(&cwd)?,
        Comando::Db {
            comando: ComandoDb::Rebuild,
        } => comandos::db_rebuild(&cwd)?,
    };
    println!("{salida}");
    Ok(())
}
