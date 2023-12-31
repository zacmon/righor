//! Contains some of the python binding that would otherwise pollute the other files.

use crate::sequence::AminoAcid;
use crate::vdj::{Model, StaticEvent};

#[cfg(all(feature = "py_binds", feature = "py_o3"))]
use pyo3::prelude::*;

use rand::rngs::SmallRng;
use rand::SeedableRng;

#[cfg_attr(all(feature = "py_binds", feature = "py_o3"), pyclass)]
pub struct Generator {
    model: Model,
    rng: SmallRng,
}

#[cfg_attr(
    all(feature = "py_binds", feature = "py_o3"),
    pyclass(get_all, set_all)
)]
#[derive(Debug, Clone)]
pub struct GenerationResult {
    pub cdr3_nt: String,
    pub cdr3_aa: Option<String>,
    pub full_seq: String,
    pub v_gene: String,
    pub j_gene: String,
    pub recombination_event: StaticEvent,
}

impl Generator {
    pub fn new(model: Model, seed: Option<u64>) -> Generator {
        let rng = match seed {
            Some(s) => SmallRng::seed_from_u64(s),
            None => SmallRng::from_entropy(),
        };

        Generator { model, rng }
    }
}

#[cfg(features = "py_binds")]
#[pymethods]
impl Generator {
    #[new]
    pub fn py_new(
        path_params: &str,
        path_marginals: &str,
        path_v_anchors: &str,
        path_j_anchors: &str,
        seed: Option<u64>,
    ) -> Generator {
        Generator::new(
            path_params,
            path_marginals,
            path_v_anchors,
            path_j_anchors,
            seed,
        )
    }
}

#[cfg_attr(features = "py_bind", pymethods)]
impl Generator {
    pub fn generate(&mut self, functional: bool) -> GenerationResult {
        let (cdr3_nt, cdr3_aa, event) = self.model.generate(functional, &mut self.rng);
        let (full_sequence, v_name, j_name) =
            self.model
                .recreate_full_sequence(&cdr3_nt, event.v_index, event.j_index);
        GenerationResult {
            full_seq: full_sequence.to_string(),
            cdr3_nt: cdr3_nt.to_string(),
            cdr3_aa: cdr3_aa.map(|x: AminoAcid| x.to_string()),
            v_gene: v_name,
            j_gene: j_name,
            recombination_event: event,
        }
    }
}
