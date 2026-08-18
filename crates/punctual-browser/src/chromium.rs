use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context as _, Result};
use chromiumoxide::{
    browser::{Browser, BrowserConfig},
    layout::Point,
    Page,
};
use chrono::Utc;
use futures::StreamExt;
use punctual_core::{ClickAttemptGuard, CompletionSignal, TargetCandidate, TargetFingerprint};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use url::Url;

use crate::{
    CandidateScorer, ClickDispatch, CompletionBaseline, CompletionVerification, CompletionVerifier,
    PageObservation, TargetProbe, DETECT_BUTTONS_SCRIPT, HIGHLIGHT_BUTTON_SCRIPT,
    PROBE_TARGET_SCRIPT,
};

pub struct ChromiumSession {
    browser: Browser,
    handler_task: JoinHandle<()>,
    scorer: CandidateScorer,
    browser_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserObservation {
    visible_text: String,
    present_selectors: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LocatorPayload<'a> {
    selector: &'a str,
    shadow_path: &'a [String],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProbePayload<'a> {
    selector: &'a str,
    shadow_path: &'a [String],
    observed_selectors: Vec<&'a str>,
    scroll_into_view: bool,
}

impl ChromiumSession {
    pub async fn launch(
        profile_dir: impl Into<PathBuf>,
        executable: Option<&Path>,
        browser_name: impl Into<String>,
    ) -> Result<Self> {
        let profile_dir = profile_dir.into();
        tokio::fs::create_dir_all(&profile_dir)
            .await
            .with_context(|| {
                format!(
                    "failed to create browser profile at {}",
                    profile_dir.display()
                )
            })?;

        let mut builder = BrowserConfig::builder()
            .with_head()
            .user_data_dir(profile_dir)
            .arg("--no-first-run")
            .arg("--no-default-browser-check");
        if let Some(executable) = executable {
            builder = builder.chrome_executable(executable);
        }

        let config = builder
            .build()
            .map_err(anyhow::Error::msg)
            .context("failed to build Chromium configuration")?;
        let (browser, mut handler) = Browser::launch(config).await?;
        let handler_task = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if event.is_err() {
                    break;
                }
            }
        });

        Ok(Self {
            browser,
            handler_task,
            scorer: CandidateScorer::default(),
            browser_name: browser_name.into(),
        })
    }

    pub fn browser_name(&self) -> &str {
        &self.browser_name
    }

    pub async fn open(&self, url: &Url) -> Result<Page> {
        self.browser
            .new_page(url.as_str())
            .await
            .with_context(|| format!("failed to open {url}"))
    }

    pub async fn detect_targets(&self, page: &Page) -> Result<Vec<TargetCandidate>> {
        let candidates = page
            .evaluate_function(DETECT_BUTTONS_SCRIPT)
            .await?
            .into_value::<Vec<TargetCandidate>>()?;
        Ok(self.scorer.infer(candidates))
    }

    pub async fn highlight(&self, page: &Page, target: &TargetFingerprint) -> Result<bool> {
        let expression = script_call(HIGHLIGHT_BUTTON_SCRIPT, target)?;
        page.evaluate_expression(expression)
            .await?
            .into_value::<bool>()
            .map_err(Into::into)
    }

    /// Scrolls the resolved target into the viewport during the pre-click Armed
    /// phase. The exact-deadline probe deliberately does not scroll.
    pub async fn prepare_target(
        &self,
        page: &Page,
        target: &TargetFingerprint,
    ) -> Result<TargetProbe> {
        let expression = probe_script_call(PROBE_TARGET_SCRIPT, target, &[], true)?;
        page.evaluate_expression(expression)
            .await?
            .into_value::<TargetProbe>()
            .map_err(Into::into)
    }

    pub async fn probe_target(
        &self,
        page: &Page,
        target: &TargetFingerprint,
    ) -> Result<TargetProbe> {
        self.probe_target_for_click(page, target, &[]).await
    }

    async fn probe_target_for_click(
        &self,
        page: &Page,
        target: &TargetFingerprint,
        signals: &[CompletionSignal],
    ) -> Result<TargetProbe> {
        let expression = probe_script_call(PROBE_TARGET_SCRIPT, target, signals, false)?;
        page.evaluate_expression(expression)
            .await?
            .into_value::<TargetProbe>()
            .map_err(Into::into)
    }

    pub async fn verify_completion(
        &self,
        source_page: &Page,
        dispatch: &ClickDispatch,
        signals: &[CompletionSignal],
    ) -> Result<CompletionVerification> {
        let mut pages = self.browser.pages().await?;
        if !pages
            .iter()
            .any(|page| target_key(page) == dispatch.source_target_key)
        {
            pages.push(source_page.clone());
        }

        let source_target_id = source_page.target_id();
        let mut candidates = pages
            .into_iter()
            .filter_map(|page| {
                let key = target_key(&page);
                let kind = if key == dispatch.source_target_key {
                    CompletionPageKind::Source
                } else if !dispatch.existing_target_keys.contains(&key) {
                    if page.opener_id().as_ref() == Some(source_target_id) {
                        CompletionPageKind::DirectNew
                    } else {
                        CompletionPageKind::New
                    }
                } else {
                    return None;
                };
                Some(CompletionPageCandidate { page, kind })
            })
            .collect::<Vec<_>>();

        // A direct popup/new tab produced by the clicked page is the strongest
        // candidate. The source page remains next so an unrelated tab opened by
        // the user cannot override a valid same-tab completion.
        candidates.sort_by_key(|candidate| candidate.kind.priority());

        let mut best_uncertain: Option<(u8, CompletionVerification)> = None;
        let mut observation_errors = Vec::new();

        for candidate in candidates {
            let observation =
                match observe_page(&candidate.page, &dispatch.completion_baseline.url, signals)
                    .await
                {
                    Ok(observation) => observation,
                    Err(error) => {
                        observation_errors.push(format!(
                            "{}读取失败：{error:#}",
                            candidate.kind.display_name()
                        ));
                        continue;
                    }
                };

            // New targets commonly begin as about:blank while navigation is
            // still being committed. Ignore that transient state and let the
            // next completion poll observe the real destination.
            if candidate.kind.is_new() && !is_meaningful_result_url(&observation.current_url) {
                continue;
            }

            let comparison_baseline = if candidate.kind == CompletionPageKind::Source {
                dispatch.completion_baseline.clone()
            } else {
                CompletionBaseline {
                    url: Url::parse("about:blank").expect("about:blank is a valid URL"),
                    visible_text: String::new(),
                    present_selectors: BTreeSet::new(),
                }
            };

            match CompletionVerifier.verify(signals, &comparison_baseline, &observation) {
                CompletionVerification::Succeeded {
                    final_url,
                    evidence,
                } => {
                    let evidence = if candidate.kind == CompletionPageKind::Source {
                        evidence
                    } else {
                        format!("{}：{evidence}", candidate.kind.display_name())
                    };
                    return Ok(CompletionVerification::Succeeded {
                        final_url,
                        evidence,
                    });
                }
                CompletionVerification::Uncertain {
                    current_url,
                    reason,
                } => {
                    let reason = if candidate.kind == CompletionPageKind::Source {
                        reason
                    } else {
                        format!(
                            "已检测到{}（{}），但尚未匹配配置的成功信号",
                            candidate.kind.display_name(),
                            current_url
                        )
                    };
                    let score = candidate.kind.uncertain_priority();
                    let candidate_result = CompletionVerification::Uncertain {
                        current_url,
                        reason,
                    };
                    if best_uncertain
                        .as_ref()
                        .is_none_or(|(best_score, _)| score < *best_score)
                    {
                        best_uncertain = Some((score, candidate_result));
                    }
                }
            }
        }

        if let Some((_, verification)) = best_uncertain {
            return Ok(verification);
        }

        let reason = if observation_errors.is_empty() {
            "点击已派发，但原标签页和点击后新建标签页均未出现可确认的结果".to_owned()
        } else {
            format!(
                "点击已派发，但无法确认结果；{}",
                observation_errors.join("；")
            )
        };
        Ok(CompletionVerification::Uncertain {
            current_url: dispatch.completion_baseline.url.clone(),
            reason,
        })
    }

    /// Dispatches exactly one native CDP mouse click after re-checking the
    /// target at the last possible moment.
    ///
    /// The process-local guard is claimed immediately before dispatch. Once it
    /// is claimed, this run will not retry the mouse event, even when the page
    /// later reports an uncertain business result.
    pub async fn click_once(
        &self,
        page: &Page,
        target: &TargetFingerprint,
        guard: &ClickAttemptGuard,
        signals: &[CompletionSignal],
    ) -> Result<ClickDispatch> {
        let probe = self.probe_target_for_click(page, target, signals).await?;
        if !probe.found {
            bail!("target was not found at click time");
        }
        if !probe.clickable {
            bail!(
                "target was not clickable at click time: {}",
                probe.reason.as_deref().unwrap_or("unknown_reason")
            );
        }
        let existing_target_keys = self
            .browser
            .pages()
            .await?
            .into_iter()
            .map(|page| target_key(&page))
            .collect::<BTreeSet<_>>();
        let source_target_key = target_key(page);

        if !guard.try_claim() {
            bail!("the logical click attempt was already claimed");
        }

        let baseline_url = probe
            .page_url
            .as_deref()
            .map(Url::parse)
            .transpose()
            .context("browser returned an invalid pre-click page URL")?
            .context("browser did not report the pre-click page URL")?;
        let completion_baseline = CompletionBaseline {
            url: baseline_url,
            visible_text: probe.visible_text,
            present_selectors: probe.present_selectors.into_iter().collect(),
        };
        let dispatched_at = Utc::now();
        page.click(Point {
            x: probe.x,
            y: probe.y,
        })
        .await?;

        Ok(ClickDispatch {
            dispatched_at,
            completion_baseline,
            source_target_key,
            existing_target_keys,
            browser_name: self.browser_name.clone(),
        })
    }

    pub async fn close(mut self) -> Result<()> {
        self.browser.close().await?;
        self.handler_task.await?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompletionPageKind {
    DirectNew,
    Source,
    New,
}

impl CompletionPageKind {
    fn priority(self) -> u8 {
        match self {
            Self::DirectNew => 0,
            Self::Source => 1,
            Self::New => 2,
        }
    }

    fn uncertain_priority(self) -> u8 {
        match self {
            Self::DirectNew => 0,
            Self::New => 1,
            Self::Source => 2,
        }
    }

    fn is_new(self) -> bool {
        !matches!(self, Self::Source)
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::DirectNew => "点击后打开的新标签页/窗口",
            Self::Source => "原标签页",
            Self::New => "点击后新建标签页/窗口",
        }
    }
}

struct CompletionPageCandidate {
    page: Page,
    kind: CompletionPageKind,
}

async fn observe_page(
    page: &Page,
    fallback_url: &Url,
    signals: &[CompletionSignal],
) -> Result<PageObservation> {
    let selectors = signals
        .iter()
        .filter_map(|signal| match signal {
            CompletionSignal::SelectorAppears { selector } => Some(selector.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let selector_json = serde_json::to_string(&selectors)?;
    let expression = format!(
        r#"(() => {{
            const selectors = {selector_json};
            const presentSelectors = selectors.filter((selector) => {{
                try {{ return document.querySelector(selector) !== null; }}
                catch (_) {{ return false; }}
            }});
            return {{
                visibleText: document.body?.innerText || "",
                presentSelectors
            }};
        }})()"#
    );
    let browser_observation = page
        .evaluate_expression(expression)
        .await?
        .into_value::<BrowserObservation>()?;
    let current_url = match page.url().await? {
        Some(value) => Url::parse(&value)
            .with_context(|| format!("browser returned an invalid page URL: {value}"))?,
        None => fallback_url.clone(),
    };

    Ok(PageObservation {
        original_url: fallback_url.clone(),
        current_url,
        visible_text: browser_observation.visible_text,
        present_selectors: browser_observation
            .present_selectors
            .into_iter()
            .collect::<BTreeSet<_>>(),
    })
}

fn target_key(page: &Page) -> String {
    format!("{:?}", page.target_id())
}

fn is_meaningful_result_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

fn script_call(script: &str, target: &TargetFingerprint) -> Result<String> {
    let selector = target
        .selector_hint
        .as_deref()
        .context("target fingerprint has no selector hint")?;
    let payload = serde_json::to_string(&LocatorPayload {
        selector,
        shadow_path: &target.shadow_path,
    })?;
    Ok(format!("({script})({payload})"))
}

fn probe_script_call(
    script: &str,
    target: &TargetFingerprint,
    signals: &[CompletionSignal],
    scroll_into_view: bool,
) -> Result<String> {
    let selector = target
        .selector_hint
        .as_deref()
        .context("target fingerprint has no selector hint")?;
    let observed_selectors = signals
        .iter()
        .filter_map(|signal| match signal {
            CompletionSignal::SelectorAppears { selector } => Some(selector.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let payload = serde_json::to_string(&ProbePayload {
        selector,
        shadow_path: &target.shadow_path,
        observed_selectors,
        scroll_into_view,
    })?;
    Ok(format!("({script})({payload})"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn serializes_locator_as_safe_javascript_data() {
        let target = TargetFingerprint {
            role: "button".into(),
            accessible_name: "购买".into(),
            visible_text: "购买".into(),
            stable_attributes: BTreeMap::new(),
            context_text: None,
            selector_hint: Some("button[data-label=\"a'b\"]".into()),
            shadow_path: vec!["#host".into()],
            frame_path: Vec::new(),
        };

        let expression = script_call("(locator) => locator", &target).unwrap();
        assert!(expression.contains("\\\"a'b\\\""));
        assert!(expression.contains("\"shadowPath\":[\"#host\"]"));
    }
}
