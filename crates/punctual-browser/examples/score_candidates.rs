use std::collections::BTreeMap;

use punctual_browser::CandidateScorer;
use punctual_core::{ElementRect, TargetCandidate};

fn main() {
    let candidates = ["加入购物车", "立即购买", "取消"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| TargetCandidate {
            candidate_id: format!("candidate-{index}"),
            tag_name: "button".into(),
            role: "button".into(),
            input_type: Some("button".into()),
            accessible_name: name.into(),
            visible_text: name.into(),
            context_text: Some("主商品区域".into()),
            selector_hint: None,
            shadow_path: Vec::new(),
            stable_attributes: BTreeMap::new(),
            rect: ElementRect {
                x: 0.0,
                y: 0.0,
                width: 120.0,
                height: 40.0,
            },
            visible: true,
            enabled: true,
            pointer_events: true,
            covered: false,
            semantic_clickable: true,
            score: 0,
            confidence: 0,
            score_reasons: vec![],
        })
        .collect();

    for candidate in CandidateScorer::default().infer(candidates) {
        println!(
            "{:>3}%  {:<12}  {}",
            candidate.confidence, candidate.best_name(), candidate.score_reasons.join("; ")
        );
    }
}
