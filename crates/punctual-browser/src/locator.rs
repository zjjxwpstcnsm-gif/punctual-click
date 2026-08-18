use punctual_core::{TargetCandidate, TargetFingerprint};

use crate::normalize_text;

/// Result of re-locating a previously selected button after the page changed.
#[derive(Debug, Clone, PartialEq)]
pub enum RelocationResult {
    Unique(RelocationMatch),
    Ambiguous(Vec<RelocationMatch>),
    NotFound,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RelocationMatch {
    pub candidate: TargetCandidate,
    pub match_score: i32,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct FingerprintLocator {
    /// Candidates within this distance of the best score are considered
    /// ambiguous and must be confirmed by the user rather than guessed.
    ambiguity_margin: i32,
    minimum_score: i32,
}


#[derive(Debug, Clone, Copy)]
enum TargetAvailability {
    ClickableNow,
    Present,
}

impl Default for FingerprintLocator {
    fn default() -> Self {
        Self {
            ambiguity_margin: 12,
            minimum_score: 70,
        }
    }
}

impl FingerprintLocator {
    pub const fn new(minimum_score: i32, ambiguity_margin: i32) -> Self {
        Self {
            ambiguity_margin,
            minimum_score,
        }
    }

    pub fn relocate(
        &self,
        fingerprint: &TargetFingerprint,
        candidates: &[TargetCandidate],
    ) -> RelocationResult {
        self.relocate_with(fingerprint, candidates, TargetAvailability::ClickableNow)
    }

    /// Relocates a target before its execution window. A matching button may
    /// still be disabled or temporarily covered at this stage; the final CDP
    /// probe remains responsible for proving that it can receive the click.
    pub fn relocate_for_execution(
        &self,
        fingerprint: &TargetFingerprint,
        candidates: &[TargetCandidate],
    ) -> RelocationResult {
        self.relocate_with(fingerprint, candidates, TargetAvailability::Present)
    }

    fn relocate_with(
        &self,
        fingerprint: &TargetFingerprint,
        candidates: &[TargetCandidate],
        availability: TargetAvailability,
    ) -> RelocationResult {
        let mut matches = candidates
            .iter()
            .filter(|candidate| match availability {
                TargetAvailability::ClickableNow => candidate.is_clickable_now(),
                TargetAvailability::Present => {
                    candidate.visible
                        && candidate.semantic_clickable
                        && candidate.rect.width > 0.0
                        && candidate.rect.height > 0.0
                }
            })
            .map(|candidate| self.score(fingerprint, candidate))
            .filter(|candidate| candidate.match_score >= self.minimum_score)
            .collect::<Vec<_>>();

        matches.sort_by(|left, right| {
            right
                .match_score
                .cmp(&left.match_score)
                .then_with(|| right.candidate.score.cmp(&left.candidate.score))
                .then_with(|| {
                    left.candidate
                        .candidate_id
                        .cmp(&right.candidate.candidate_id)
                })
        });

        let Some(best) = matches.first() else {
            return RelocationResult::NotFound;
        };
        let cutoff = best.match_score - self.ambiguity_margin;
        let plausible = matches
            .into_iter()
            .take_while(|value| value.match_score >= cutoff)
            .collect::<Vec<_>>();

        if plausible.len() == 1 {
            RelocationResult::Unique(plausible.into_iter().next().expect("one match exists"))
        } else {
            RelocationResult::Ambiguous(plausible)
        }
    }

    fn score(
        &self,
        fingerprint: &TargetFingerprint,
        candidate: &TargetCandidate,
    ) -> RelocationMatch {
        let mut score = 0;
        let mut reasons = Vec::new();

        if fingerprint.shadow_path == candidate.shadow_path {
            score += 35;
            reasons.push("Shadow DOM 路径一致 +35".into());
        } else if !fingerprint.shadow_path.is_empty() || !candidate.shadow_path.is_empty() {
            score -= 50;
            reasons.push("Shadow DOM 路径不一致 -50".into());
        }

        if let (Some(expected), Some(actual)) = (
            fingerprint.selector_hint.as_deref(),
            candidate.selector_hint.as_deref(),
        ) {
            if expected == actual {
                score += 45;
                reasons.push("选择器提示一致 +45".into());
            }
        }

        if !fingerprint.role.is_empty()
            && normalize_text(&fingerprint.role) == normalize_text(&candidate.role)
        {
            score += 18;
            reasons.push("可访问性角色一致 +18".into());
        }

        let expected_accessible_name = normalize_text(&fingerprint.accessible_name);
        let actual_accessible_name = normalize_text(&candidate.accessible_name);
        if !expected_accessible_name.is_empty()
            && expected_accessible_name == actual_accessible_name
        {
            score += 75;
            reasons.push("可访问名称精确一致 +75".into());
        }

        let expected_visible_text = normalize_text(&fingerprint.visible_text);
        let actual_visible_text = normalize_text(&candidate.visible_text);
        if !expected_visible_text.is_empty() && expected_visible_text == actual_visible_text {
            score += 50;
            reasons.push("可见文案精确一致 +50".into());
        }

        let mut stable_matches = 0;
        let mut stable_conflicts = 0;
        for (name, expected) in &fingerprint.stable_attributes {
            match candidate.stable_attributes.get(name) {
                Some(actual) if actual == expected => stable_matches += 1,
                Some(_) => stable_conflicts += 1,
                None => {}
            }
        }
        if stable_matches > 0 {
            let points = stable_matches * 80;
            score += points;
            reasons.push(format!("{stable_matches} 个稳定属性一致 +{points}"));
        }
        if stable_conflicts > 0 {
            let points = stable_conflicts * 90;
            score -= points;
            reasons.push(format!("{stable_conflicts} 个稳定属性冲突 -{points}"));
        }

        if let (Some(expected), Some(actual)) = (
            fingerprint.context_text.as_deref(),
            candidate.context_text.as_deref(),
        ) {
            let expected = normalize_text(expected);
            let actual = normalize_text(actual);
            if !expected.is_empty() && !actual.is_empty() {
                if expected == actual {
                    score += 30;
                    reasons.push("上下文精确一致 +30".into());
                } else if expected.contains(&actual) || actual.contains(&expected) {
                    score += 16;
                    reasons.push("上下文高度重合 +16".into());
                }
            }
        }

        RelocationMatch {
            candidate: candidate.clone(),
            match_score: score,
            reasons,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use punctual_core::{ElementRect, TargetCandidate, TargetFingerprint};

    use super::*;

    fn candidate(id: &str, text: &str, context: &str) -> TargetCandidate {
        TargetCandidate {
            candidate_id: id.into(),
            tag_name: "button".into(),
            role: "button".into(),
            input_type: Some("button".into()),
            accessible_name: text.into(),
            visible_text: text.into(),
            context_text: Some(context.into()),
            selector_hint: Some(format!("#{id}")),
            shadow_path: Vec::new(),
            stable_attributes: BTreeMap::from([("data-testid".into(), id.into())]),
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
            score: 100,
            confidence: 90,
            score_reasons: Vec::new(),
        }
    }

    fn fingerprint() -> TargetFingerprint {
        TargetFingerprint {
            role: "button".into(),
            accessible_name: "购买".into(),
            visible_text: "购买".into(),
            stable_attributes: BTreeMap::from([("data-testid".into(), "main-buy".into())]),
            context_text: Some("主商品 iPhone".into()),
            selector_hint: Some("#main-buy".into()),
            shadow_path: Vec::new(),
            frame_path: Vec::new(),
        }
    }

    #[test]
    fn stable_attribute_selects_main_button() {
        let result = FingerprintLocator::default().relocate(
            &fingerprint(),
            &[
                candidate("recommend-buy", "购买", "猜你喜欢"),
                candidate("main-buy", "购买", "主商品 iPhone"),
            ],
        );

        assert!(matches!(
            result,
            RelocationResult::Unique(value)
                if value.candidate.candidate_id == "main-buy"
        ));
    }

    #[test]
    fn requires_choice_when_identical_targets_remain() {
        let mut fingerprint = fingerprint();
        fingerprint.selector_hint = None;
        fingerprint.stable_attributes.clear();
        fingerprint.context_text = None;

        let mut first = candidate("a", "购买", "商品");
        let mut second = candidate("b", "购买", "商品");
        first.selector_hint = None;
        second.selector_hint = None;
        first.stable_attributes.clear();
        second.stable_attributes.clear();

        let result = FingerprintLocator::default().relocate(&fingerprint, &[first, second]);
        assert!(matches!(result, RelocationResult::Ambiguous(values) if values.len() == 2));
    }

    #[test]
    fn ignores_non_clickable_candidate() {
        let mut disabled = candidate("main-buy", "购买", "主商品 iPhone");
        disabled.enabled = false;

        let result = FingerprintLocator::default().relocate(&fingerprint(), &[disabled]);
        assert_eq!(result, RelocationResult::NotFound);
    }
    #[test]
    fn execution_relocation_keeps_temporarily_disabled_target() {
        let mut disabled = candidate("main-buy", "购买", "主商品 iPhone");
        disabled.enabled = false;

        let result = FingerprintLocator::default()
            .relocate_for_execution(&fingerprint(), &[disabled]);
        assert!(matches!(result, RelocationResult::Unique(_)));
    }

}
