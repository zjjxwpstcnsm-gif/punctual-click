use punctual_core::TargetCandidate;

use crate::normalize_text;

#[derive(Debug, Clone, PartialEq)]
pub enum ManualValidation {
    Unique(TargetCandidate),
    Multiple(Vec<TargetCandidate>),
    NotClickable(Vec<TargetCandidate>),
    NotFound,
}

impl ManualValidation {
    pub const fn is_valid(&self) -> bool {
        matches!(self, Self::Unique(_))
    }
}

pub fn validate_manual_text(
    requested_text: &str,
    candidates: &[TargetCandidate],
) -> ManualValidation {
    let query = normalize_text(requested_text);
    if query.is_empty() {
        return ManualValidation::NotFound;
    }

    let matching = candidates
        .iter()
        .filter(|candidate| {
            normalize_text(&candidate.accessible_name) == query
                || normalize_text(&candidate.visible_text) == query
        })
        .cloned()
        .collect::<Vec<_>>();

    let clickable = matching
        .iter()
        .filter(|candidate| candidate.is_clickable_now())
        .cloned()
        .collect::<Vec<_>>();

    match clickable.len() {
        1 => ManualValidation::Unique(clickable.into_iter().next().unwrap()),
        2.. => ManualValidation::Multiple(clickable),
        _ if !matching.is_empty() => ManualValidation::NotClickable(matching),
        _ => ManualValidation::NotFound,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use punctual_core::{ElementRect, TargetCandidate};

    use super::*;

    fn candidate(id: &str, name: &str, enabled: bool) -> TargetCandidate {
        TargetCandidate {
            candidate_id: id.into(),
            tag_name: "button".into(),
            role: "button".into(),
            input_type: None,
            accessible_name: name.into(),
            visible_text: name.into(),
            context_text: None,
            selector_hint: None,
            shadow_path: Vec::new(),
            stable_attributes: BTreeMap::new(),
            rect: ElementRect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 40.0,
            },
            visible: true,
            enabled,
            pointer_events: true,
            covered: false,
            semantic_clickable: true,
            score: 0,
            confidence: 0,
            score_reasons: vec![],
        }
    }

    #[test]
    fn returns_unique_clickable_match() {
        let result = validate_manual_text("提交订单", &[candidate("a", "提交订单", true)]);
        assert!(matches!(result, ManualValidation::Unique(_)));
    }

    #[test]
    fn requires_choice_for_duplicate_buttons() {
        let result = validate_manual_text(
            "购买",
            &[
                candidate("main", "购买", true),
                candidate("recommendation", "购买", true),
            ],
        );
        assert!(matches!(result, ManualValidation::Multiple(values) if values.len() == 2));
    }

    #[test]
    fn distinguishes_text_found_but_disabled() {
        let result = validate_manual_text("结算", &[candidate("a", "结算", false)]);
        assert!(matches!(result, ManualValidation::NotClickable(_)));
    }
}
