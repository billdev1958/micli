//! Propiedades del trace: cualquier secuencia de payloads produce una
//! cadena verificable, y la cadena sobrevive a reaperturas arbitrarias.

use micli_trace::{Payload, Trace};
use proptest::prelude::*;

fn payload_arbitrario() -> impl Strategy<Value = Payload> {
    ("[a-zA-Z0-9-]{1,12}", ".{0,40}").prop_map(|(id, titulo)| Payload::RequirementCreado {
        id: micli_core::RequirementId::new(id),
        titulo,
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Toda secuencia de appends deja una cadena que verifica completa.
    #[test]
    fn toda_secuencia_de_appends_verifica(
        payloads in prop::collection::vec(payload_arbitrario(), 0..20),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let mut trace = Trace::abrir(dir.path().join("trace.jsonl")).unwrap();
        let total = payloads.len() as u64;
        for payload in payloads {
            trace.append(payload).unwrap();
        }
        prop_assert_eq!(trace.verificar().unwrap(), total);
    }

    /// Reabrir el trace en cualquier punto no rompe la cadena.
    #[test]
    fn reaperturas_intermedias_no_rompen_la_cadena(
        lotes in prop::collection::vec(
            prop::collection::vec(payload_arbitrario(), 0..5),
            1..5,
        ),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trace.jsonl");
        let total: u64 = lotes.iter().map(|l| l.len() as u64).sum();
        for lote in lotes {
            let mut trace = Trace::abrir(&path).unwrap();
            for payload in lote {
                trace.append(payload).unwrap();
            }
        }
        prop_assert_eq!(Trace::abrir(&path).unwrap().verificar().unwrap(), total);
    }
}
