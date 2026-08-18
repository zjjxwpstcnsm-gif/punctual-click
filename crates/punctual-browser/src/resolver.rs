use punctual_core::TargetCandidate;
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone)]
pub struct CandidateScorer {
    action_terms: Vec<String>,
    negative_terms: Vec<String>,
}

impl Default for CandidateScorer {
    fn default() -> Self {
        Self::new(
            [
                "立即购买",
                "购买",
                "抢购",
                "提交",
                "提交订单",
                "结算",
                "去结算",
                "确认",
                "确认提交",
                "支付",
                "立即支付",
                "预约",
                "立即预约",
                "报名",
                "buy now",
                "purchase",
                "submit",
                "checkout",
                "place order",
                "confirm",
                "pay now",
                "reserve",
                "book now",
                "register",
            ],
            [
                "取消", "返回", "关闭", "删除", "稍后", "cancel", "back", "close", "delete",
                "later",
            ],
        )
    }
}

impl CandidateScorer {
    pub fn new<A, N, AS, NS>(action_terms: A, negative_terms: N) -> Self
    where
        A: IntoIterator<Item = AS>,
        AS: Into<String>,
        N: IntoIterator<Item = NS>,
        NS: Into<String>,
    {
        Self {
            action_terms: action_terms
                .into_iter()
                .map(|value| normalize_text(&value.into()))
                .collect(),
            negative_terms: negative_terms
                .into_iter()
                .map(|value| normalize_text(&value.into()))
                .collect(),
        }
    }

    pub fn infer(&self, mut candidates: Vec<TargetCandidate>) -> Vec<TargetCandidate> {
        for candidate in &mut candidates {
            self.score(candidate);
        }
        candidates.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| right.confidence.cmp(&left.confidence))
                .then_with(|| left.candidate_id.cmp(&right.candidate_id))
        });
        candidates
    }

    pub fn score(&self, candidate: &mut TargetCandidate) {
        let mut score = 0_i32;
        let mut reasons = Vec::new();

        if !candidate.visible {
            reasons.push("元素当前不可见".into());
            candidate.score = -1_000;
            candidate.confidence = 0;
            candidate.score_reasons = reasons;
            return;
        }
        if !candidate.enabled {
            reasons.push("元素当前处于禁用状态".into());
            candidate.score = -900;
            candidate.confidence = 0;
            candidate.score_reasons = reasons;
            return;
        }
        if !candidate.pointer_events {
            reasons.push("pointer-events 禁止接收点击".into());
            candidate.score = -850;
            candidate.confidence = 0;
            candidate.score_reasons = reasons;
            return;
        }
        if candidate.covered {
            reasons.push("元素中心点被其他元素遮挡".into());
            candidate.score = -800;
            candidate.confidence = 0;
            candidate.score_reasons = reasons;
            return;
        }
        if !candidate.semantic_clickable {
            reasons.push("元素缺少按钮、链接或提交控件语义".into());
            candidate.score = -700;
            candidate.confidence = 0;
            candidate.score_reasons = reasons;
            return;
        }

        let primary_name = normalize_text(candidate.best_name());
        let context = normalize_text(candidate.context_text.as_deref().unwrap_or_default());
        let role = normalize_text(&candidate.role);
        let tag = normalize_text(&candidate.tag_name);
        let input_type = normalize_text(candidate.input_type.as_deref().unwrap_or_default());

        if role == "button" {
            score += 20;
            reasons.push("可访问性角色为 button +20".into());
        }
        if tag == "button" {
            score += 20;
            reasons.push("原生 button 元素 +20".into());
        }
        if input_type == "submit" {
            score += 20;
            reasons.push("表单 submit 控件 +20".into());
        }
        if candidate
            .stable_attributes
            .keys()
            .any(|key| matches!(key.as_str(), "id" | "data-testid" | "data-test" | "name"))
        {
            score += 8;
            reasons.push("存在稳定定位属性 +8".into());
        }

        let mut best_action_score = 0;
        let mut matched_action = None;
        for term in &self.action_terms {
            let term_score = if primary_name == *term {
                100
            } else if !primary_name.is_empty() && primary_name.contains(term) {
                55
            } else if !context.is_empty() && context.contains(term) {
                15
            } else {
                0
            };
            if term_score > best_action_score {
                best_action_score = term_score;
                matched_action = Some(term.as_str());
            }
        }
        if let Some(term) = matched_action {
            score += best_action_score;
            reasons.push(format!("匹配动作词“{term}” +{best_action_score}"));
        }

        for term in &self.negative_terms {
            if primary_name == *term || primary_name.contains(term) {
                score -= 120;
                reasons.push(format!("匹配排除词“{term}” -120"));
                break;
            }
        }

        if contains_any(
            &context,
            &["商品", "订单", "购物车", "结算", "product", "order", "cart"],
        ) {
            score += 10;
            reasons.push("上下文位于商品、订单或结算区域 +10".into());
        }

        if candidate.rect.width >= 44.0 && candidate.rect.height >= 28.0 {
            score += 4;
            reasons.push("控件尺寸适合直接点击 +4".into());
        }

        candidate.score = score;
        candidate.confidence = score_to_confidence(score);
        candidate.score_reasons = reasons;
    }
}

pub fn normalize_text(value: &str) -> String {
    value
        .nfkc()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_lowercase()
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn score_to_confidence(score: i32) -> u8 {
    if score <= 0 {
        return 0;
    }
    ((score as f64 / 160.0) * 100.0).round().clamp(1.0, 99.0) as u8
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use punctual_core::ElementRect;

    use super::*;

    fn candidate(name: &str) -> TargetCandidate {
        TargetCandidate {
            candidate_id: name.into(),
            tag_name: "button".into(),
            role: "button".into(),
            input_type: Some("button".into()),
            accessible_name: name.into(),
            visible_text: name.into(),
            context_text: Some("主商品区域 iPhone 订单".into()),
            selector_hint: Some("#buy".into()),
            shadow_path: Vec::new(),
            stable_attributes: BTreeMap::from([("id".into(), "buy".into())]),
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
        }
    }

    #[test]
    fn ranks_purchase_above_cancel() {
        let scorer = CandidateScorer::default();
        let ranked = scorer.infer(vec![candidate("取消"), candidate("立即购买")]);
        assert_eq!(ranked[0].accessible_name, "立即购买");
        assert!(ranked[0].confidence > ranked[1].confidence);
    }

    #[test]
    fn disqualifies_covered_element() {
        let scorer = CandidateScorer::default();
        let mut value = candidate("提交订单");
        value.covered = true;
        scorer.score(&mut value);
        assert_eq!(value.confidence, 0);
        assert!(value.score < 0);
    }

    #[test]
    fn normalizes_full_width_and_whitespace() {
        assert_eq!(normalize_text("  Ｂｕｙ   Ｎｏｗ  "), "buy now");
    }
}
