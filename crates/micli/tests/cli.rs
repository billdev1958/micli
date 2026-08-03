//! Tests de integración del binario: se ejecuta el exe real
//! (`CARGO_BIN_EXE_micli`) contra directorios temporales, con `MICLI_HOME`
//! apuntando a un ámbito global aislado por test.
//!
//! `allow-unwrap-in-tests` de clippy.toml solo cubre funciones `#[test]`;
//! los helpers de este archivo también son código de test.
#![allow(clippy::unwrap_used)]

use micli_core::{Event, RequirementId, State};
use micli_trace::{Payload, Trace};
use std::path::Path;
use std::process::Output;

fn micli(proyecto: &Path, global: &Path, args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_micli"))
        .args(args)
        .current_dir(proyecto)
        .env("MICLI_HOME", global)
        .output()
        .unwrap()
}

fn stdout(salida: &Output) -> String {
    String::from_utf8_lossy(&salida.stdout).into_owned()
}

fn stderr(salida: &Output) -> String {
    String::from_utf8_lossy(&salida.stderr).into_owned()
}

/// Crea proyecto + ámbito global temporales y corre `init` con perfil rust.
fn proyecto_inicializado() -> (tempfile::TempDir, tempfile::TempDir) {
    let proyecto = tempfile::tempdir().unwrap();
    let global = tempfile::tempdir().unwrap();
    let salida = micli(
        proyecto.path(),
        global.path(),
        &["init", "--profile", "rust", "--name", "demo"],
    );
    assert!(salida.status.success(), "init falló: {}", stderr(&salida));
    (proyecto, global)
}

#[test]
fn init_genera_manifests_y_registra_globalmente() {
    let (proyecto, global) = proyecto_inicializado();
    let dir = proyecto.path().join(".micli");
    assert!(dir.join("agent_rules.toml").exists());
    assert!(dir.join("requirements.toml").exists());
    assert!(dir.join("trace.jsonl").exists());

    let registro = std::fs::read_to_string(global.path().join("projects.toml")).unwrap();
    assert!(registro.contains("demo"));

    // Reintentar el init falla: el proyecto ya está registrado.
    let repetido = micli(
        proyecto.path(),
        global.path(),
        &["init", "--profile", "rust"],
    );
    assert!(!repetido.status.success());
    assert!(stderr(&repetido).contains("ya está registrado"));
}

#[test]
fn status_sin_init_explica_como_empezar() {
    let proyecto = tempfile::tempdir().unwrap();
    let global = tempfile::tempdir().unwrap();
    let salida = micli(proyecto.path(), global.path(), &["status"]);
    assert!(!salida.status.success());
    assert!(stderr(&salida).contains("micli init"));
}

#[test]
fn status_muestra_estados_derivados_del_trace() {
    let (proyecto, global) = proyecto_inicializado();
    let dir = proyecto.path().join(".micli");

    std::fs::write(
        dir.join("requirements.toml"),
        r#"
            version = 1
            [[requirements]]
            id = "REQ-001"
            titulo = "Alta de usuarios"
            [[requirements]]
            id = "REQ-002"
            titulo = "Login"
        "#,
    )
    .unwrap();

    let mut trace = Trace::abrir(dir.join("trace.jsonl")).unwrap();
    trace
        .append(Payload::RequirementCreado {
            id: RequirementId::new("REQ-001"),
            titulo: "Alta de usuarios".to_owned(),
        })
        .unwrap();
    trace
        .append(Payload::Transicion {
            requirement_id: RequirementId::new("REQ-001"),
            evento: Event::AprobarSpec,
            desde: State::Draft,
            hacia: State::SpecAprobada,
        })
        .unwrap();

    let salida = micli(proyecto.path(), global.path(), &["status"]);
    assert!(salida.status.success(), "{}", stderr(&salida));
    let texto = stdout(&salida);
    assert!(texto.contains("demo"));
    assert!(texto.contains("2 eventos"));
    assert!(texto.contains("REQ-001"));
    assert!(texto.contains("SpecAprobada"));
    // REQ-002 no tiene eventos: declarado en Draft.
    assert!(texto.contains("REQ-002"));
    assert!(texto.contains("Draft"));
}

#[test]
fn trace_verify_detecta_alteraciones() {
    let (proyecto, global) = proyecto_inicializado();
    let dir = proyecto.path().join(".micli");

    let mut trace = Trace::abrir(dir.join("trace.jsonl")).unwrap();
    for n in 0..3 {
        trace
            .append(Payload::RequirementCreado {
                id: RequirementId::new(format!("REQ-{n:03}")),
                titulo: format!("Requerimiento {n}"),
            })
            .unwrap();
    }

    let ok = micli(proyecto.path(), global.path(), &["trace", "verify"]);
    assert!(ok.status.success(), "{}", stderr(&ok));
    assert!(stdout(&ok).contains("3 eventos"));

    // Alteración externa de un evento pasado.
    let ruta = dir.join("trace.jsonl");
    let contenido = std::fs::read_to_string(&ruta).unwrap();
    std::fs::write(
        &ruta,
        contenido.replace("Requerimiento 1", "Requerimiento X"),
    )
    .unwrap();

    let roto = micli(proyecto.path(), global.path(), &["trace", "verify"]);
    assert!(!roto.status.success());
    assert!(stderr(&roto).contains("alterada"));
}

#[test]
fn dos_proyectos_conviven_en_el_mismo_registro_global() {
    let global = tempfile::tempdir().unwrap();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let init_a = micli(
        a.path(),
        global.path(),
        &["init", "--profile", "rust", "--name", "a"],
    );
    assert!(init_a.status.success(), "{}", stderr(&init_a));
    let init_b = micli(
        b.path(),
        global.path(),
        &["init", "--profile", "python-fastapi", "--name", "b"],
    );
    assert!(init_b.status.success(), "{}", stderr(&init_b));

    let registro = std::fs::read_to_string(global.path().join("projects.toml")).unwrap();
    assert!(registro.contains("\"a\"") || registro.contains("= \"a\""));
    assert!(registro.contains("python") || registro.contains("b"));
}
