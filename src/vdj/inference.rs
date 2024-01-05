use crate::shared::feature::*;
use crate::shared::utils::{InferenceParameters, RangeArray2};
use crate::vdj::{AggregatedFeatureDJ, AggregatedFeatureEndV, Model, Sequence};
use anyhow::Result;
#[cfg(all(feature = "py_binds", feature = "pyo3"))]
use pyo3::{pyclass, pymethods};
use std::cmp;

#[derive(Default, Clone, Debug)]
#[cfg_attr(all(feature = "py_binds", feature = "pyo3"), pyclass(get_all))]
pub struct Features {
    pub v: CategoricalFeature1,
    pub delv: CategoricalFeature1g1,
    pub dj: CategoricalFeature2,
    pub delj: CategoricalFeature1g1,
    pub deld: CategoricalFeature2g1,
    // pub nb_insvd: CategoricalFeature1,
    // pub nb_insdj: CategoricalFeature1,
    pub insvd: InsertionFeature,
    pub insdj: InsertionFeature,
    pub error: ErrorSingleNucleotide,
}

impl Features {
    pub fn new(model: &Model) -> Result<Features> {
        Ok(Features {
            v: CategoricalFeature1::new(&model.p_v)?,
            delv: CategoricalFeature1g1::new(&model.p_del_v_given_v)?,
            dj: CategoricalFeature2::new(&model.p_dj)?,
            delj: CategoricalFeature1g1::new(&model.p_del_j_given_j)?,
            deld: CategoricalFeature2g1::new(&model.p_del_d3_del_d5)?, // dim: (d3, d5, d)
            insvd: InsertionFeature::new(
                &model.p_ins_vd,
                &model.first_nt_bias_ins_vd,
                &model.markov_coefficients_vd,
            )?,
            insdj: InsertionFeature::new(
                &model.p_ins_dj,
                &model.first_nt_bias_ins_dj,
                &model.markov_coefficients_dj,
            )?,
            error: ErrorSingleNucleotide::new(model.error_rate)?,
        })
    }

    pub fn infer(&mut self, sequence: &Sequence, ip: &InferenceParameters) -> Result<f64> {
        // let mut probability_generation: f64 = 0.; // need to deal with that
        // let mut probability_generation_no_error: f64 = 0.; // TODO return this too
        // let mut best_events = Vec::<(f64, StaticEvent)>::new();

        let mut feature_v = AggregatedFeatureEndV::new(sequence, &self, ip);
        let mut feature_dj = AggregatedFeatureDJ::new(sequence, &self, ip);

        let mut ll_ins_vd = Vec::new();
        for ev in feature_v.start_v3..feature_v.end_v3 {
            for sd in cmp::max(ev, feature_dj.start_d5)..feature_dj.end_d5 {
                if sd - ev <= self.insvd.max_nb_insertions() as i64 {
                    let ins_vd = sequence.get_subsequence(ev, sd);
                    ll_ins_vd.push(((ev, sd), self.insvd.log_likelihood(&ins_vd)));
                }
            }
        }
        let log_likelihood_ins_vd = RangeArray2::new(&ll_ins_vd);

        let mut ll_ins_dj = Vec::new();
        for sj in feature_dj.start_j5..feature_dj.end_j5 {
            for ed in feature_dj.start_d3..cmp::min(feature_dj.end_d3, sj + 1) {
                if sj - ed <= self.insdj.max_nb_insertions() as i64 {
                    // careful we need to reverse ins_dj for the inference
                    let mut ins_dj = sequence.get_subsequence(ed, sj);
                    ins_dj.reverse();
                    ll_ins_dj.push(((ed, sj), self.insdj.log_likelihood(&ins_dj)));
                }
            }
        }

        let log_likelihood_ins_dj = RangeArray2::new(&ll_ins_dj);

        let mut l_total = 0.;

        for ev in feature_v.start_v3..feature_v.end_v3 {
            for sd in cmp::max(ev, feature_dj.start_d5)..feature_dj.end_d5 {
                if sd - ev > self.insvd.max_nb_insertions() as i64 {
                    continue;
                }
                let mut likelihood_v = 0.;
                let ins_vd = sequence.get_subsequence(ev, sd);
                let log_likelihood_v = feature_v.log_likelihood(ev);
                if log_likelihood_ins_vd.get((ev, sd)) < ip.min_log_likelihood {
                    continue;
                }
                for ed in cmp::max(sd - 1, feature_dj.start_d3)..feature_dj.end_d3 {
                    for sj in cmp::max(ed, feature_dj.start_j5)..feature_dj.end_j5 {
                        if sj - ed > self.insdj.max_nb_insertions() as i64 {
                            continue;
                        }
                        let mut likelihood_ins = 0.;
                        let mut ins_dj = sequence.get_subsequence(ed, sj);
                        ins_dj.reverse();
                        let log_likelihood_ins = log_likelihood_ins_vd.get((ev, sd))
                            + log_likelihood_ins_dj.get((ed, sj));
                        for j_idx in 0..feature_dj.nb_j_alignments {
                            let log_likelihood = feature_dj.log_likelihood(sd, ed, sj, j_idx)
                                + log_likelihood_ins
                                + log_likelihood_v;

                            if log_likelihood > ip.min_log_likelihood {
                                let likelihood = log_likelihood.exp2();
                                likelihood_v += likelihood;
                                likelihood_ins += likelihood;
                                l_total += likelihood;
                                feature_dj.dirty_update(sd, ed, sj, j_idx, likelihood);
                            }
                        }
                        if likelihood_ins > 0. {
                            self.insvd.dirty_update(&ins_vd, likelihood_ins);
                            self.insdj.dirty_update(&ins_dj, likelihood_ins);
                        }
                    }
                }
                if likelihood_v > 0. {
                    feature_v.dirty_update(ev, likelihood_v);
                }
            }
        }

        if l_total != 0. {
            feature_v.cleanup(&sequence, self, ip);
            feature_dj.cleanup(&sequence, self, ip);
            *self = self.cleanup()?;
        }

        Ok(l_total)
    }

    // fn log_likelihood_estimate_post_v(&self, e: &Event, m: &Model) -> f64 {
    //     self.v.log_likelihood(e.v.unwrap().index) + m.max_log_likelihood_post_v
    // }

    // fn log_likelihood_estimate_post_delv(&self, e: &Event, m: &Model) -> f64 {
    //     self.v.log_likelihood(e.v.unwrap().index)
    //         + self.delv.log_likelihood((e.delv, e.v.unwrap().index))
    //         + self.error.log_likelihood((
    //             e.v.unwrap().nb_errors(e.delv),
    //             e.v.unwrap().length_with_deletion(e.delv),
    //         ))
    //         + m.max_log_likelihood_post_delv
    // }

    // fn log_likelihood_estimate_post_dj(&self, e: &Event, m: &Model) -> f64 {
    //     let v = e.v.unwrap();
    //     let d = e.d.unwrap();
    //     let j = e.j.unwrap();
    //     let v_end = difference_as_i64(v.end_seq, e.delv);

    //     self.v.log_likelihood(v.index)
    //         + self.dj.log_likelihood((d.index, j.index))
    //         + self.delv.log_likelihood((e.delv, v.index))
    //         + self.error.log_likelihood((v.nb_errors(e.delv), v.length_with_deletion(e.delv)))
    //     // We can already compute a fairly precise estimate of the maximum by looking at
    //     // the expected number of insertions
    // 	    + m.max_log_likelihood_post_dj(d.index, (j.start_seq as i64) - v_end)
    // }

    // fn log_likelihood_estimate_post_delj(&self, e: &Event, m: &Model) -> f64 {
    //     let v = e.v.unwrap();
    //     let d = e.d.unwrap();
    //     let j = e.j.unwrap();

    //     let v_end = difference_as_i64(v.end_seq, e.delv);
    //     let j_start = (j.start_seq + e.delj) as i64;

    //     if v_end > j_start {
    //         return f64::NEG_INFINITY;
    //     }

    //     self.v.log_likelihood(v.index)
    //         + self.delv.log_likelihood((e.delv, v.index))
    //         + self.dj.log_likelihood((d.index, j.index))
    //         + self
    //             .error
    //             .log_likelihood((v.nb_errors(e.delv), v.length_with_deletion(e.delv)))
    //         + self.delj.log_likelihood((e.delj, j.index))
    //         + self
    //             .error
    //             .log_likelihood((j.nb_errors(e.delj), j.length_with_deletion(e.delj)))
    //         + m.max_log_likelihood_post_delj(d.index, (j_start - v_end) as usize)
    // }

    // fn log_likelihood_estimate_post_deld(&self, e: &Event, m: &Model) -> f64 {
    //     let v = e.v.unwrap();
    //     let d = e.d.unwrap();
    //     let j = e.j.unwrap();

    //     let v_end = difference_as_i64(v.end_seq, e.delv);
    //     let d_start = (d.pos + e.deld5) as i64;
    //     let d_end = (d.pos + d.len() - e.deld3) as i64;
    //     let j_start = (j.start_seq + e.delj) as i64;

    //     if (v_end > d_start) || (d_start > d_end) || (d_end > j_start) {
    //         return f64::NEG_INFINITY;
    //     }

    //     self.v.log_likelihood(v.index)
    //         + self.delv.log_likelihood((e.delv, v.index))
    //         + self.dj.log_likelihood((d.index, j.index))
    //         + self
    //             .error
    //             .log_likelihood((v.nb_errors(e.delv), v.length_with_deletion(e.delv)))
    //         + self.delj.log_likelihood((e.delj, j.index))
    //         + self
    //             .error
    //             .log_likelihood((j.nb_errors(e.delj), j.length_with_deletion(e.delj)))
    //         + self.deld.log_likelihood((e.deld3, e.deld5, d.index))
    //         + self.error.log_likelihood((
    //             d.nb_errors(e.deld5, e.deld3),
    //             d.length_with_deletion(e.deld5, e.deld3),
    //         ))
    //         + m.max_log_likelihood_post_deld((d_start - v_end) as usize, (j_start - d_end) as usize)
    // }

    // fn log_likelihood(&self, e: &Event, ins_vd: &Dna, ins_dj: &Dna) -> f64 {
    //     let v = e.v.unwrap();
    //     let d = e.d.unwrap();
    //     let j = e.j.unwrap();

    //     self.v.log_likelihood(v.index)
    //         + self.delv.log_likelihood((e.delv, v.index))
    //         + self.dj.log_likelihood((d.index, j.index))
    //         + self
    //             .error
    //             .log_likelihood((v.nb_errors(e.delv), v.length_with_deletion(e.delv)))
    //         + self.delj.log_likelihood((e.delj, j.index))
    //         + self
    //             .error
    //             .log_likelihood((j.nb_errors(e.delj), j.length_with_deletion(e.delj)))
    //         + self.deld.log_likelihood((e.deld3, e.deld5, d.index))
    //         + self.error.log_likelihood((
    //             d.nb_errors(e.deld5, e.deld3),
    //             d.length_with_deletion(e.deld5, e.deld3),
    //         ))
    //         + self.insvd.log_likelihood(ins_vd)
    //         + self.insdj.log_likelihood(ins_dj)
    // }

    // // fn log_likelihood_no_error(&self, e: &Event, ins_vd: &Dna, ins_dj: &Dna) -> f64 {
    // //     let v = e.v.unwrap();
    // //     let d = e.d.unwrap();
    // //     let j = e.j.unwrap();
    // //     return self.v.log_likelihood(v.index)
    // //         + self.delv.log_likelihood((e.delv, v.index))
    // //         + self.dj.log_likelihood((d.index, j.index))
    // //         + self.delj.log_likelihood((e.delj, j.index))
    // //         + self.insvd.log_likelihood(ins_vd)
    // //         + self.insdj.log_likelihood(ins_dj);
    // // }

    // pub fn dirty_update(&mut self, e: &Event, likelihood: f64, insvd: &Dna, insdj: &Dna) {
    //     let v = e.v.unwrap();
    //     let d = e.d.unwrap();
    //     let j = e.j.unwrap();
    //     self.v.dirty_update(v.index, likelihood);
    //     self.dj.dirty_update((d.index, j.index), likelihood);
    //     self.delv.dirty_update((e.delv, v.index), likelihood);
    //     self.delj.dirty_update((e.delj, j.index), likelihood);
    //     self.deld
    //         .dirty_update((e.deld3, e.deld5, d.index), likelihood);
    //     self.insvd.dirty_update(insvd, likelihood);
    //     self.insdj.dirty_update(insdj, likelihood);
    //     self.error.dirty_update(
    //         (
    //             j.nb_errors(e.delj) + v.nb_errors(e.delv) + d.nb_errors(e.deld5, e.deld3),
    //             j.length_with_deletion(e.delj)
    //                 + v.length_with_deletion(e.delv)
    //                 + d.length_with_deletion(e.deld5, e.deld3),
    //         ),
    //         likelihood,
    //     );
    // }

    // pub fn infer(
    //     &mut self,
    //     sequence: &Sequence,
    //     m: &Model,
    //     ip: &InferenceParameters,
    // ) -> (f64, Vec<(f64, StaticEvent)>) {
    //     let mut probability_generation: f64 = 0.;
    //     // let mut probability_generation_no_error: f64 = 0.; // TODO return this too
    //     let mut best_events = Vec::<(f64, StaticEvent)>::new();

    //     for v in sequence.v_genes.iter() {
    //         let ev = Event {
    //             v: Some(v),
    //             ..Event::default()
    //         };
    //         if self.log_likelihood_estimate_post_v(&ev, m) < ip.min_log_likelihood {
    //             continue;
    //         }
    //         for delv in 0..self.delv.dim().0 {
    //             let edelv = Event {
    //                 v: Some(v),
    //                 delv,
    //                 ..Event::default()
    //             };
    //             if self.log_likelihood_estimate_post_delv(&edelv, m) < ip.min_log_likelihood {
    //                 continue;
    //             }

    //             for (j, d) in iproduct!(sequence.j_genes.iter(), sequence.d_genes.iter()) {
    //                 let edj = Event {
    //                     v: Some(v),
    //                     delv,
    //                     d: Some(d),
    //                     j: Some(j),
    //                     ..Event::default()
    //                 };
    //                 if self.log_likelihood_estimate_post_dj(&edj, m) < ip.min_log_likelihood {
    //                     continue;
    //                 }
    //                 for delj in 0..self.delj.dim().0 {
    //                     let edelj = Event {
    //                         v: Some(v),
    //                         delv,
    //                         d: Some(d),
    //                         j: Some(j),
    //                         delj,
    //                         ..Event::default()
    //                     };
    //                     if self.log_likelihood_estimate_post_delj(&edelj, m) < ip.min_log_likelihood
    //                     {
    //                         continue;
    //                     }
    //                     for (deld5, deld3) in iproduct!(0..self.deld.dim().1, 0..self.deld.dim().0)
    //                     {
    //                         // println!("deld5 {}, deld3 {}", self.deld.dim().1, self.deld.dim().0)
    //                         let efinal = Event {
    //                             v: Some(v),
    //                             delv,
    //                             d: Some(d),
    //                             j: Some(j),
    //                             delj,
    //                             deld5,
    //                             deld3,
    //                         };
    //                         if self.log_likelihood_estimate_post_deld(&efinal, m)
    //                             < ip.min_log_likelihood
    //                         {
    //                             continue;
    //                         }
    //                         // otherwise compute the real likelihood
    //                         let (insvd, insdj) = sequence.get_insertions_vd_dj(&efinal);
    //                         // println!("{}", sequence.sequence.get_string());
    //                         // println!("{}", efinal.v.unwrap().end_seq);
    //                         // println!("{}", efinal.delv);
    //                         //println!("{}", insvd.get_string());

    //                         let mut insdj_reversed = insdj.clone();
    //                         // careful insdj must be reversed (supposed to start on the J side)
    //                         insdj_reversed.reverse();
    //                         let log_likelihood =
    //                             self.log_likelihood(&efinal, &insvd, &insdj_reversed);
    //                         if log_likelihood < ip.min_log_likelihood {
    //                             continue;
    //                         }
    //                         let likelihood = log_likelihood.exp2();

    //                         probability_generation += likelihood;
    //                         self.dirty_update(&efinal, likelihood, &insvd, &insdj_reversed);
    //                         // println!("{}", insvd.get_string());

    //                         if ip.evaluate && ip.nb_best_events > 0 {
    //                             // probability_generation_no_error +=
    //                             //     self.likelihood_no_error(&e, &insvd, &insdj);
    //                             if (best_events.len() < ip.nb_best_events)
    //                                 || (best_events.last().unwrap().0 < likelihood)
    //                             {
    //                                 best_events = insert_in_order(
    //                                     best_events,
    //                                     (
    //                                         likelihood,
    //                                         efinal.to_static(insvd.clone(), insdj.clone()).unwrap(),
    //                                     ),
    //                                 );
    //                                 best_events.truncate(ip.nb_best_events);
    //                             }
    //                         }
    //                     }
    //                 }
    //             }
    //         }
    //     }
    //     (probability_generation, best_events)
    // }

    pub fn cleanup(&self) -> Result<Features> {
        // Compute the new marginals for the next round
        Ok(Features {
            v: self.v.cleanup()?,
            dj: self.dj.cleanup()?,
            delv: self.delv.cleanup()?,
            delj: self.delj.cleanup()?,
            deld: self.deld.cleanup()?,
            insvd: self.insvd.cleanup()?,
            insdj: self.insdj.cleanup()?,
            error: self.error.cleanup()?,
        })
    }
}

impl Features {
    pub fn average(features: Vec<Features>) -> Result<Features> {
        Ok(Features {
            v: CategoricalFeature1::average(features.iter().map(|a| a.v.clone()))?,
            delv: CategoricalFeature1g1::average(features.iter().map(|a| a.delv.clone()))?,
            dj: CategoricalFeature2::average(features.iter().map(|a| a.dj.clone()))?,
            delj: CategoricalFeature1g1::average(features.iter().map(|a| a.delj.clone()))?,
            deld: CategoricalFeature2g1::average(features.iter().map(|a| a.deld.clone()))?,
            insvd: InsertionFeature::average(features.iter().map(|a| a.insvd.clone()))?,
            insdj: InsertionFeature::average(features.iter().map(|a| a.insdj.clone()))?,
            error: ErrorSingleNucleotide::average(features.iter().map(|a| a.error.clone()))?,
        })
    }
}

#[cfg(all(feature = "py_binds", feature = "pyo3"))]
#[pymethods]
impl Features {
    #[staticmethod]
    #[pyo3(name = "average")]
    pub fn py_average(features: Vec<Features>) -> Result<Features> {
        Features::average(features)
    }
}
