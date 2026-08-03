//! Cálculo y render del avance: lógica pura, sin E/S, para poder testearla
//! sin proyectos reales.

use micli_core::State;
use std::collections::BTreeMap;

/// Módulo bajo el que se agrupan los requerimientos sin módulo declarado.
pub const MODULO_GENERAL: &str = "general";

/// Conteo de requerimientos por situación.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Avance {
    pub total: usize,
    pub aceptados: usize,
    pub bloqueados: usize,
    pub en_curso: usize,
}

impl Avance {
    pub fn desde(estados: impl IntoIterator<Item = State>) -> Self {
        let mut avance = Self::default();
        for estado in estados {
            avance.total += 1;
            match estado {
                State::Aceptado => avance.aceptados += 1,
                State::Bloqueado { .. } => avance.bloqueados += 1,
                _ => avance.en_curso += 1,
            }
        }
        avance
    }

    /// Porcentaje de aceptados (0–100, truncado). Sin requerimientos = 0.
    pub fn porcentaje(&self) -> usize {
        (self.aceptados * 100).checked_div(self.total).unwrap_or(0)
    }
}

/// Barra de progreso ASCII de `ancho` celdas: `#` aceptado, `!` bloqueado,
/// `-` en curso.
pub fn barra(avance: &Avance, ancho: usize) -> String {
    if avance.total == 0 {
        return format!("[{}]", "-".repeat(ancho));
    }
    let llenos = avance.aceptados * ancho / avance.total;
    let bloqueados = avance.bloqueados * ancho / avance.total;
    let resto = ancho.saturating_sub(llenos + bloqueados);
    format!(
        "[{}{}{}]",
        "#".repeat(llenos),
        "!".repeat(bloqueados),
        "-".repeat(resto)
    )
}

/// Agrupa avance por módulo, ordenado alfabéticamente. Un módulo vacío se
/// agrupa como [`MODULO_GENERAL`].
pub fn por_modulo<'a>(
    filas: impl IntoIterator<Item = (&'a str, State)>,
) -> BTreeMap<String, Avance> {
    let mut grupos: BTreeMap<String, Vec<State>> = BTreeMap::new();
    for (modulo, estado) in filas {
        let clave = if modulo.trim().is_empty() {
            MODULO_GENERAL.to_owned()
        } else {
            modulo.to_owned()
        };
        grupos.entry(clave).or_default().push(estado);
    }
    grupos
        .into_iter()
        .map(|(modulo, estados)| (modulo, Avance::desde(estados)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use micli_core::ActiveState;

    #[test]
    fn cuenta_aceptados_bloqueados_y_en_curso() {
        let avance = Avance::desde([
            State::Aceptado,
            State::Aceptado,
            State::Bloqueado {
                desde: ActiveState::Implementando,
            },
            State::Draft,
        ]);
        assert_eq!(avance.total, 4);
        assert_eq!(avance.aceptados, 2);
        assert_eq!(avance.bloqueados, 1);
        assert_eq!(avance.en_curso, 1);
        assert_eq!(avance.porcentaje(), 50);
    }

    #[test]
    fn sin_requerimientos_el_porcentaje_es_cero_y_la_barra_vacia() {
        let avance = Avance::default();
        assert_eq!(avance.porcentaje(), 0);
        assert_eq!(barra(&avance, 10), "[----------]");
    }

    #[test]
    fn la_barra_siempre_mide_el_ancho_pedido() {
        for (aceptados, bloqueados, en_curso) in [(0, 0, 1), (1, 0, 0), (1, 1, 1), (3, 2, 5)] {
            let mut estados = vec![State::Aceptado; aceptados];
            estados.extend(vec![
                State::Bloqueado {
                    desde: ActiveState::Draft,
                };
                bloqueados
            ]);
            estados.extend(vec![State::Draft; en_curso]);
            let avance = Avance::desde(estados);
            let render = barra(&avance, 20);
            assert_eq!(render.chars().count(), 22, "{render}");
        }
        let completo = Avance::desde([State::Aceptado, State::Aceptado]);
        assert_eq!(barra(&completo, 10), "[##########]");
    }

    #[test]
    fn agrupa_por_modulo_con_general_para_vacios() {
        let grupos = por_modulo([
            ("auth", State::Aceptado),
            ("auth", State::Draft),
            ("", State::Aceptado),
            ("  ", State::Draft),
        ]);
        assert_eq!(grupos.len(), 2);
        assert_eq!(grupos["auth"].total, 2);
        assert_eq!(grupos["auth"].aceptados, 1);
        assert_eq!(grupos[MODULO_GENERAL].total, 2);
    }
}
