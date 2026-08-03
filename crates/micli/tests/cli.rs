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
    assert!(dir.join("prompts").join("sistema.md").exists());

    let registro = std::fs::read_to_string(global.path().join("projects.toml")).unwrap();
    assert!(registro.contains("demo"));
    // La ruta registrada es legible: sin el prefijo \\?\ de canonicalize().
    assert!(!registro.contains('?'), "{registro}");

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
            modulo = "auth"
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
    // Avance global y desglose por módulo.
    assert!(texto.contains("Avance:"), "{texto}");
    assert!(texto.contains("Por módulo:"), "{texto}");
    assert!(texto.contains("auth"), "{texto}");
    assert!(texto.contains("general"), "{texto}");
}

#[test]
fn dashboard_muestra_todos_los_proyectos_y_tolera_rutas_perdidas() {
    let global = tempfile::tempdir().unwrap();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    for (dir, nombre) in [(&a, "alfa"), (&b, "beta")] {
        let salida = micli(
            dir.path(),
            global.path(),
            &["init", "--profile", "rust", "--name", nombre],
        );
        assert!(salida.status.success(), "{}", stderr(&salida));
    }
    // alfa consigue un requerimiento aceptado vía trace directo.
    std::fs::write(
        a.path().join(".micli").join("requirements.toml"),
        r#"
            version = 1
            [[requirements]]
            id = "REQ-001"
            titulo = "Alta"
        "#,
    )
    .unwrap();
    let mut trace = Trace::abrir(a.path().join(".micli").join("trace.jsonl")).unwrap();
    trace
        .append(Payload::RequirementCreado {
            id: RequirementId::new("REQ-001"),
            titulo: "Alta".to_owned(),
        })
        .unwrap();

    let salida = micli(a.path(), global.path(), &["dashboard"]);
    assert!(salida.status.success(), "{}", stderr(&salida));
    let texto = stdout(&salida);
    assert!(texto.contains("Proyectos registrados: 2"), "{texto}");
    assert!(texto.contains("alfa"), "{texto}");
    assert!(texto.contains("beta"), "{texto}");
    assert!(texto.contains('%'), "{texto}");

    // Si el directorio de un proyecto desaparece, el dashboard no revienta.
    let ruta_beta = b.path().to_path_buf();
    drop(b);
    assert!(!ruta_beta.exists());
    let salida = micli(a.path(), global.path(), &["dashboard"]);
    assert!(salida.status.success(), "{}", stderr(&salida));
    let texto = stdout(&salida);
    assert!(texto.contains("no accesible"), "{texto}");
    assert!(texto.contains("alfa"), "{texto}");
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
fn el_ciclo_req_add_y_avanzar_funciona_por_terminal() {
    let (proyecto, global) = proyecto_inicializado();
    std::fs::write(
        proyecto.path().join(".micli").join("requirements.toml"),
        r#"
            version = 1
            [[requirements]]
            id = "REQ-001"
            titulo = "Alta de usuarios"
            spec = "Dado un correo valido, crear el usuario."
        "#,
    )
    .unwrap();

    let add = micli(proyecto.path(), global.path(), &["req", "add", "REQ-001"]);
    assert!(add.status.success(), "{}", stderr(&add));
    assert!(stdout(&add).contains("Draft"));

    let avanzar = micli(
        proyecto.path(),
        global.path(),
        &["req", "avanzar", "REQ-001", "--evento", "aprobar-spec"],
    );
    assert!(avanzar.status.success(), "{}", stderr(&avanzar));
    assert!(stdout(&avanzar).contains("SpecAprobada"));

    // Un evento inválido para el estado actual falla con explicación.
    let invalido = micli(
        proyecto.path(),
        global.path(),
        &["req", "avanzar", "REQ-001", "--evento", "aprobar-spec"],
    );
    assert!(!invalido.status.success());
    assert!(stderr(&invalido).contains("transición rechazada"));

    // run exige TestRojo (falla antes de invocar agente alguno).
    let run = micli(proyecto.path(), global.path(), &["run", "REQ-001"]);
    assert!(!run.status.success());
    assert!(stderr(&run).contains("TestRojo"));

    // db rebuild proyecta lo que hay.
    let rebuild = micli(proyecto.path(), global.path(), &["db", "rebuild"]);
    assert!(rebuild.status.success(), "{}", stderr(&rebuild));
    assert!(proyecto.path().join(".micli").join("micli.db").exists());
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
