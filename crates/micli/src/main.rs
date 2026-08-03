//! Binario CLI de `micli`: parsing de args, wiring de los crates y salida a
//! terminal. Único crate del workspace con permiso de imprimir.
#![allow(clippy::print_stdout)]

mod comandos;
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
    /// Operaciones sobre el log de eventos
    Trace {
        #[command(subcommand)]
        comando: ComandoTrace,
    },
}

#[derive(Subcommand)]
enum ComandoTrace {
    /// Verifica la integridad de la cadena de eventos
    Verify,
}

/// Espejo de `manifest::Perfil` para clap (kebab-case en la terminal).
#[derive(Debug, Clone, Copy, ValueEnum)]
enum PerfilArg {
    Rust,
    Angular,
    PythonFastapi,
    Node,
}

impl From<PerfilArg> for Perfil {
    fn from(arg: PerfilArg) -> Self {
        match arg {
            PerfilArg::Rust => Self::Rust,
            PerfilArg::Angular => Self::Angular,
            PerfilArg::PythonFastapi => Self::PythonFastapi,
            PerfilArg::Node => Self::Node,
        }
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir()?;
    let salida = match cli.comando {
        Comando::Init { profile, name } => comandos::init(&cwd, name, profile.into())?,
        Comando::Status => comandos::status(&cwd)?,
        Comando::Trace {
            comando: ComandoTrace::Verify,
        } => comandos::trace_verify(&cwd)?,
    };
    println!("{salida}");
    Ok(())
}
