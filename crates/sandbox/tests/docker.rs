//! Tests de integración contra un Docker real. Están `#[ignore]` porque la
//! suite debe pasar en máquinas sin Docker; se corren manualmente con:
//!
//! ```text
//! docker pull alpine:3
//! cargo test -p micli-sandbox -- --ignored
//! ```
#![allow(clippy::unwrap_used)]

use micli_sandbox::{ConfigSandbox, Red, Sandbox};
use std::time::Duration;

fn sandbox(dir: &std::path::Path, timeout: Duration) -> Sandbox {
    Sandbox::nuevo(ConfigSandbox {
        imagen: "alpine:3".to_owned(),
        red: Red::None,
        dir_proyecto: dir.to_path_buf(),
        timeout,
    })
    .unwrap()
}

#[test]
#[ignore = "requiere Docker corriendo y la imagen alpine:3"]
fn ejecuta_y_captura_salida_y_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let sandbox = sandbox(dir.path(), Duration::from_secs(60));

    let ok = sandbox.ejecutar("echo hola desde el sandbox").unwrap();
    assert!(ok.paso(), "{ok:?}");
    assert!(ok.salida.contains("hola desde el sandbox"));

    let fallo = sandbox.ejecutar("exit 7").unwrap();
    assert!(!fallo.paso());
    assert_eq!(fallo.exit_code, Some(7));
}

#[test]
#[ignore = "requiere Docker corriendo y la imagen alpine:3"]
fn el_workspace_es_el_proyecto_montado() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("marcador.txt"), "presente").unwrap();
    let sandbox = sandbox(dir.path(), Duration::from_secs(60));

    let resultado = sandbox.ejecutar("cat marcador.txt").unwrap();
    assert!(resultado.paso(), "{resultado:?}");
    assert!(resultado.salida.contains("presente"));
}

#[test]
#[ignore = "requiere Docker corriendo y la imagen alpine:3"]
fn el_timeout_mata_el_contenedor() {
    let dir = tempfile::tempdir().unwrap();
    let sandbox = sandbox(dir.path(), Duration::from_secs(2));

    let resultado = sandbox.ejecutar("sleep 60").unwrap();
    assert!(resultado.timeout_agotado);
    assert!(!resultado.paso());
}

#[test]
#[ignore = "requiere Docker corriendo y la imagen alpine:3"]
fn sin_red_el_contenedor_no_ve_el_exterior() {
    let dir = tempfile::tempdir().unwrap();
    let sandbox = sandbox(dir.path(), Duration::from_secs(30));

    // Sin red, la resolución/el socket fallan y wget devuelve exit != 0.
    let resultado = sandbox
        .ejecutar("wget -T 3 -q -O - http://example.com")
        .unwrap();
    assert!(!resultado.paso(), "{resultado:?}");
}
