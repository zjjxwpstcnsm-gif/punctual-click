use std::collections::BTreeSet;

use punctual_core::CompletionSignal;
use url::Url;

use crate::normalize_text;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageObservation {
    pub original_url: Url,
    pub current_url: Url,
    pub visible_text: String,
    pub present_selectors: BTreeSet<String>,
}

/// Page state captured by the same target probe that immediately precedes the
/// native click. Success signals must represent a transition away from this
/// baseline; a message or selector that was already present is not evidence
/// that the click succeeded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionBaseline {
    pub url: Url,
    pub visible_text: String,
    pub present_selectors: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionVerification {
    Succeeded { final_url: Url, evidence: String },
    Uncertain { current_url: Url, reason: String },
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CompletionVerifier;

impl CompletionVerifier {
    pub fn verify(
        &self,
        signals: &[CompletionSignal],
        baseline: &CompletionBaseline,
        observation: &PageObservation,
    ) -> CompletionVerification {
        let baseline_text = normalize_text(&baseline.visible_text);
        let current_text = normalize_text(&observation.visible_text);

        for signal in signals {
            let evidence = match signal {
                CompletionSignal::UrlChanged if observation.current_url != baseline.url => {
                    Some("页面链接已经变化".to_owned())
                }
                CompletionSignal::UrlMatches { pattern }
                    if !pattern.trim().is_empty()
                        && observation.current_url.as_str().contains(pattern.trim()) =>
                {
                    (!baseline.url.as_str().contains(pattern.trim()))
                        .then(|| format!("结果链接新匹配“{pattern}”"))
                }
                CompletionSignal::TextAppears { text }
                    if !normalize_text(text).is_empty()
                        && current_text.contains(&normalize_text(text)) =>
                {
                    (!baseline_text.contains(&normalize_text(text)))
                        .then(|| format!("页面新出现“{text}”"))
                }
                CompletionSignal::SelectorAppears { selector }
                    if observation.present_selectors.contains(selector) =>
                {
                    (!baseline.present_selectors.contains(selector))
                        .then(|| format!("页面新出现选择器“{selector}”"))
                }
                _ => None,
            };

            if let Some(evidence) = evidence {
                return CompletionVerification::Succeeded {
                    final_url: observation.current_url.clone(),
                    evidence,
                };
            }
        }

        CompletionVerification::Uncertain {
            current_url: observation.current_url.clone(),
            reason: "点击已派发，但尚未观察到配置的成功信号".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn baseline() -> CompletionBaseline {
        CompletionBaseline {
            url: Url::parse("https://example.com/checkout").unwrap(),
            visible_text: "订单确认".into(),
            present_selectors: BTreeSet::new(),
        }
    }

    fn observation(current: &str) -> PageObservation {
        PageObservation {
            original_url: Url::parse("https://example.com/checkout").unwrap(),
            current_url: Url::parse(current).unwrap(),
            visible_text: "订单提交成功".into(),
            present_selectors: BTreeSet::from(["[data-state='success']".into()]),
        }
    }

    #[test]
    fn accepts_changed_result_url() {
        let result = CompletionVerifier.verify(
            &[CompletionSignal::UrlChanged],
            &baseline(),
            &observation("https://example.com/order/42"),
        );
        assert!(matches!(
            result,
            CompletionVerification::Succeeded { final_url, .. }
                if final_url.as_str() == "https://example.com/order/42"
        ));
    }

    #[test]
    fn accepts_text_signal_for_single_page_app() {
        let result = CompletionVerifier.verify(
            &[CompletionSignal::TextAppears {
                text: "提交成功".into(),
            }],
            &baseline(),
            &observation("https://example.com/checkout"),
        );
        assert!(matches!(result, CompletionVerification::Succeeded { .. }));
    }

    #[test]
    fn stays_uncertain_without_evidence() {
        let result = CompletionVerifier.verify(
            &[CompletionSignal::TextAppears {
                text: "支付完成".into(),
            }],
            &baseline(),
            &observation("https://example.com/checkout"),
        );
        assert!(matches!(result, CompletionVerification::Uncertain { .. }));
    }

    #[test]
    fn ignores_success_text_that_was_already_present_before_click() {
        let baseline = CompletionBaseline {
            url: Url::parse("https://example.com/checkout").unwrap(),
            visible_text: "订单提交成功".into(),
            present_selectors: BTreeSet::new(),
        };
        let result = CompletionVerifier.verify(
            &[CompletionSignal::TextAppears {
                text: "订单提交成功".into(),
            }],
            &baseline,
            &observation("https://example.com/checkout"),
        );
        assert!(matches!(result, CompletionVerification::Uncertain { .. }));
    }
}
