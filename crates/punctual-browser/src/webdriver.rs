use std::{
    collections::BTreeSet,
    net::TcpListener,
    path::PathBuf,
    process::Stdio,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use anyhow::{bail, Context as _, Result};
use chrono::Utc;
use punctual_core::{ClickAttemptGuard, CompletionSignal, TargetCandidate, TargetFingerprint};
use reqwest::{Client, Method, Response};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{
    process::{Child, Command},
    time::sleep,
};
use url::Url;

use crate::{
    BrowserInstallation, BrowserKind, CandidateScorer, ClickDispatch, CompletionBaseline,
    CompletionVerification, CompletionVerifier, PageObservation, TargetProbe,
    DETECT_BUTTONS_SCRIPT, HIGHLIGHT_BUTTON_SCRIPT, PROBE_TARGET_SCRIPT,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebDriverPage {
    pub(crate) handle: String,
}

pub struct WebDriverSession {
    client: Client,
    endpoint: String,
    session_id: String,
    driver: Child,
    scorer: CandidateScorer,
    browser_name: String,
    kind: BrowserKind,
    first_window_claimed: AtomicBool,
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

impl WebDriverSession {
    pub async fn launch(installation: &BrowserInstallation, profile_dir: PathBuf) -> Result<Self> {
        let driver_path = installation
            .driver
            .as_deref()
            .with_context(|| webdriver_missing_driver_message(installation.kind))?;
        let executable = installation.executable.as_deref();

        tokio::fs::create_dir_all(&profile_dir)
            .await
            .with_context(|| {
                format!(
                    "failed to create browser profile at {}",
                    profile_dir.display()
                )
            })?;

        let port = available_local_port()?;
        let endpoint = format!("http://127.0.0.1:{port}");
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("failed to create WebDriver HTTP client")?;

        let mut command = Command::new(driver_path);
        command
            .arg("--port")
            .arg(port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut driver = command
            .spawn()
            .with_context(|| format!("failed to start WebDriver at {}", driver_path.display()))?;

        if let Err(error) =
            wait_until_ready(&client, &endpoint, &mut driver, installation.kind).await
        {
            let _ = driver.start_kill();
            let _ = driver.wait().await;
            return Err(error);
        }

        let always_match = match installation.kind {
            BrowserKind::Safari => json!({
                "browserName": "safari",
                "pageLoadStrategy": "eager"
            }),
            BrowserKind::Firefox => {
                let executable = executable.context("Firefox executable path is missing")?;
                json!({
                    "browserName": "firefox",
                    "pageLoadStrategy": "eager",
                    "moz:firefoxOptions": {
                        "binary": executable.to_string_lossy(),
                        "args": ["-profile", profile_dir.to_string_lossy()]
                    }
                })
            }
            _ => bail!(
                "WebDriver backend does not support {}",
                installation.display_name()
            ),
        };

        let session_response = client
            .post(format!("{endpoint}/session"))
            .json(&json!({
                "capabilities": {
                    "alwaysMatch": always_match
                }
            }))
            .send()
            .await
            .context("failed to create WebDriver session")?;
        let payload = parse_response(session_response)
            .await
            .with_context(|| webdriver_session_hint(installation.kind))?;
        let session_id = payload
            .get("sessionId")
            .and_then(Value::as_str)
            .or_else(|| {
                payload
                    .get("value")
                    .and_then(|value| value.get("sessionId"))
                    .and_then(Value::as_str)
            })
            .or_else(|| payload.get("session_id").and_then(Value::as_str))
            .context("WebDriver did not return a session id")?
            .to_owned();

        Ok(Self {
            client,
            endpoint,
            session_id,
            driver,
            scorer: CandidateScorer::default(),
            browser_name: installation.display_name().to_owned(),
            kind: installation.kind,
            first_window_claimed: AtomicBool::new(false),
        })
    }

    pub fn browser_name(&self) -> &str {
        &self.browser_name
    }

    pub async fn open(&self, url: &Url) -> Result<WebDriverPage> {
        let handle = if !self.first_window_claimed.swap(true, Ordering::AcqRel) {
            self.current_window_handle().await?
        } else {
            self.create_window().await?
        };
        let page = WebDriverPage { handle };
        self.switch_to(&page).await?;
        self.command(Method::POST, "/url", Some(json!({ "url": url.as_str() })))
            .await
            .with_context(|| format!("failed to open {url}"))?;
        Ok(page)
    }

    pub async fn detect_targets(&self, page: &WebDriverPage) -> Result<Vec<TargetCandidate>> {
        let script = format!("return ({DETECT_BUTTONS_SCRIPT})();");
        let value = self.execute(page, &script, Vec::new()).await?;
        let candidates = serde_json::from_value::<Vec<TargetCandidate>>(value)
            .context("WebDriver returned invalid target candidates")?;
        Ok(self.scorer.infer(candidates))
    }

    pub async fn highlight(
        &self,
        page: &WebDriverPage,
        target: &TargetFingerprint,
    ) -> Result<bool> {
        let payload = locator_payload(target)?;
        let script = format!("return ({HIGHLIGHT_BUTTON_SCRIPT})(arguments[0]);");
        let value = self.execute(page, &script, vec![payload]).await?;
        serde_json::from_value(value).context("WebDriver returned an invalid highlight result")
    }

    pub async fn prepare_target(
        &self,
        page: &WebDriverPage,
        target: &TargetFingerprint,
    ) -> Result<TargetProbe> {
        self.probe_target_for_click(page, target, &[], true).await
    }

    pub async fn probe_target(
        &self,
        page: &WebDriverPage,
        target: &TargetFingerprint,
    ) -> Result<TargetProbe> {
        self.probe_target_for_click(page, target, &[], false).await
    }

    async fn probe_target_for_click(
        &self,
        page: &WebDriverPage,
        target: &TargetFingerprint,
        signals: &[CompletionSignal],
        scroll_into_view: bool,
    ) -> Result<TargetProbe> {
        let payload = probe_payload(target, signals, scroll_into_view)?;
        let script = format!("return ({PROBE_TARGET_SCRIPT})(arguments[0]);");
        let value = self.execute(page, &script, vec![payload]).await?;
        serde_json::from_value(value).context("WebDriver returned an invalid target probe")
    }

    pub async fn click_once(
        &self,
        page: &WebDriverPage,
        target: &TargetFingerprint,
        guard: &ClickAttemptGuard,
        signals: &[CompletionSignal],
    ) -> Result<ClickDispatch> {
        let probe = self
            .probe_target_for_click(page, target, signals, false)
            .await?;
        if !probe.found {
            bail!("target was not found at click time");
        }
        if !probe.clickable {
            bail!(
                "target was not clickable at click time: {}",
                probe.reason.as_deref().unwrap_or("unknown_reason")
            );
        }

        let existing_target_keys = self.window_handles().await?.into_iter().collect();
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

        self.switch_to(page).await?;
        let dispatched_at = Utc::now();
        self.command(
            Method::POST,
            "/actions",
            Some(json!({
                "actions": [{
                    "type": "pointer",
                    "id": "punctual-mouse",
                    "parameters": { "pointerType": "mouse" },
                    "actions": [
                        {
                            "type": "pointerMove",
                            "duration": 0,
                            "origin": "viewport",
                            "x": probe.x.round() as i64,
                            "y": probe.y.round() as i64
                        },
                        { "type": "pointerDown", "button": 0 },
                        { "type": "pointerUp", "button": 0 }
                    ]
                }]
            })),
        )
        .await?;
        let _ = self.command(Method::DELETE, "/actions", None).await;

        Ok(ClickDispatch {
            dispatched_at,
            completion_baseline,
            source_target_key: page.handle.clone(),
            existing_target_keys,
            browser_name: self.browser_name.clone(),
        })
    }

    pub async fn verify_completion(
        &self,
        source_page: &WebDriverPage,
        dispatch: &ClickDispatch,
        signals: &[CompletionSignal],
    ) -> Result<CompletionVerification> {
        let handles = self.window_handles().await?;
        let mut candidates = handles
            .into_iter()
            .filter_map(|handle| {
                if handle == dispatch.source_target_key {
                    Some((CompletionPageKind::Source, WebDriverPage { handle }))
                } else if !dispatch.existing_target_keys.contains(&handle) {
                    Some((CompletionPageKind::New, WebDriverPage { handle }))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if !candidates.iter().any(|(_, page)| page == source_page) {
            candidates.push((CompletionPageKind::Source, source_page.clone()));
        }
        candidates.sort_by_key(|(kind, _)| kind.priority());

        let mut best_uncertain: Option<(u8, CompletionVerification)> = None;
        let mut observation_errors = Vec::new();

        for (kind, page) in candidates {
            let observation = match self
                .observe_page(&page, &dispatch.completion_baseline.url, signals)
                .await
            {
                Ok(observation) => observation,
                Err(error) => {
                    observation_errors.push(format!("{}读取失败：{error:#}", kind.display_name()));
                    continue;
                }
            };
            if kind.is_new() && !is_meaningful_result_url(&observation.current_url) {
                continue;
            }

            let baseline = if kind == CompletionPageKind::Source {
                dispatch.completion_baseline.clone()
            } else {
                CompletionBaseline {
                    url: Url::parse("about:blank").expect("about:blank is valid"),
                    visible_text: String::new(),
                    present_selectors: BTreeSet::new(),
                }
            };

            match CompletionVerifier.verify(signals, &baseline, &observation) {
                CompletionVerification::Succeeded {
                    final_url,
                    evidence,
                } => {
                    return Ok(CompletionVerification::Succeeded {
                        final_url,
                        evidence: if kind == CompletionPageKind::Source {
                            evidence
                        } else {
                            format!("{}：{evidence}", kind.display_name())
                        },
                    });
                }
                CompletionVerification::Uncertain {
                    current_url,
                    reason,
                } => {
                    let reason = if kind == CompletionPageKind::Source {
                        reason
                    } else {
                        format!(
                            "已检测到{}（{}），但尚未匹配配置的成功信号",
                            kind.display_name(),
                            current_url
                        )
                    };
                    let candidate = CompletionVerification::Uncertain {
                        current_url,
                        reason,
                    };
                    if best_uncertain
                        .as_ref()
                        .is_none_or(|(score, _)| kind.uncertain_priority() < *score)
                    {
                        best_uncertain = Some((kind.uncertain_priority(), candidate));
                    }
                }
            }
        }

        if let Some((_, verification)) = best_uncertain {
            return Ok(verification);
        }
        let reason = if observation_errors.is_empty() {
            "点击已派发，但原标签页和点击后新建标签页均未出现可确认的结果".into()
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

    pub async fn current_url(&self, page: &WebDriverPage) -> Result<Url> {
        self.switch_to(page).await?;
        let value = self.command(Method::GET, "/url", None).await?;
        let raw = value
            .as_str()
            .context("WebDriver returned a non-string page URL")?;
        Url::parse(raw).with_context(|| format!("WebDriver returned an invalid page URL: {raw}"))
    }

    async fn observe_page(
        &self,
        page: &WebDriverPage,
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
        let script = r#"
            const selectors = arguments[0] || [];
            const presentSelectors = selectors.filter((selector) => {
                try { return document.querySelector(selector) !== null; }
                catch (_) { return false; }
            });
            return {
                visibleText: document.body?.innerText || "",
                presentSelectors
            };
        "#;
        let value = self
            .execute(page, script, vec![serde_json::to_value(selectors)?])
            .await?;
        let browser_observation = serde_json::from_value::<BrowserObservation>(value)
            .context("WebDriver returned an invalid page observation")?;
        let current_url = self
            .current_url(page)
            .await
            .unwrap_or_else(|_| fallback_url.clone());
        Ok(PageObservation {
            original_url: fallback_url.clone(),
            current_url,
            visible_text: browser_observation.visible_text,
            present_selectors: browser_observation.present_selectors.into_iter().collect(),
        })
    }

    async fn execute(&self, page: &WebDriverPage, script: &str, args: Vec<Value>) -> Result<Value> {
        self.switch_to(page).await?;
        self.command(
            Method::POST,
            "/execute/sync",
            Some(json!({ "script": script, "args": args })),
        )
        .await
    }

    async fn create_window(&self) -> Result<String> {
        let before = self
            .window_handles()
            .await?
            .into_iter()
            .collect::<BTreeSet<_>>();
        match self
            .command(Method::POST, "/window/new", Some(json!({ "type": "tab" })))
            .await
        {
            Ok(value) => {
                if let Some(handle) = value.get("handle").and_then(Value::as_str) {
                    return Ok(handle.to_owned());
                }
            }
            Err(_) => {
                // Older Safari implementations may not expose New Window. A
                // standards-compatible script fallback still produces a handle
                // that can be discovered through Get Window Handles.
                let current = WebDriverPage {
                    handle: self.current_window_handle().await?,
                };
                let _ = self
                    .execute(
                        &current,
                        "window.open('about:blank', '_blank'); return true;",
                        Vec::new(),
                    )
                    .await?;
            }
        }

        for _ in 0..40 {
            if let Some(handle) = self
                .window_handles()
                .await?
                .into_iter()
                .find(|handle| !before.contains(handle))
            {
                return Ok(handle);
            }
            sleep(Duration::from_millis(50)).await;
        }
        bail!("WebDriver did not create a new browser tab")
    }

    async fn switch_to(&self, page: &WebDriverPage) -> Result<()> {
        self.command(
            Method::POST,
            "/window",
            Some(json!({ "handle": page.handle })),
        )
        .await?;
        Ok(())
    }

    async fn current_window_handle(&self) -> Result<String> {
        let value = self.command(Method::GET, "/window", None).await?;
        value
            .as_str()
            .map(ToOwned::to_owned)
            .context("WebDriver returned an invalid current window handle")
    }

    async fn window_handles(&self) -> Result<Vec<String>> {
        let value = self.command(Method::GET, "/window/handles", None).await?;
        serde_json::from_value(value).context("WebDriver returned invalid window handles")
    }

    async fn command(&self, method: Method, suffix: &str, body: Option<Value>) -> Result<Value> {
        let url = format!("{}/session/{}{}", self.endpoint, self.session_id, suffix);
        let request = self.client.request(method, url);
        let response = match body {
            Some(body) => request.json(&body).send().await?,
            None => request.send().await?,
        };
        parse_response(response).await
    }

    pub async fn close(mut self) -> Result<()> {
        let _ = self
            .client
            .delete(format!("{}/session/{}", self.endpoint, self.session_id))
            .send()
            .await;
        if self.driver.try_wait()?.is_none() {
            let _ = self.driver.start_kill();
        }
        let _ = self.driver.wait().await;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompletionPageKind {
    New,
    Source,
}

impl CompletionPageKind {
    const fn priority(self) -> u8 {
        match self {
            Self::New => 0,
            Self::Source => 1,
        }
    }

    const fn uncertain_priority(self) -> u8 {
        self.priority()
    }

    const fn is_new(self) -> bool {
        matches!(self, Self::New)
    }

    const fn display_name(self) -> &'static str {
        match self {
            Self::New => "点击后打开的新标签页/窗口",
            Self::Source => "原标签页",
        }
    }
}

fn locator_payload(target: &TargetFingerprint) -> Result<Value> {
    let selector = target
        .selector_hint
        .as_deref()
        .context("target fingerprint has no selector hint")?;
    serde_json::to_value(LocatorPayload {
        selector,
        shadow_path: &target.shadow_path,
    })
    .map_err(Into::into)
}

fn probe_payload(
    target: &TargetFingerprint,
    signals: &[CompletionSignal],
    scroll_into_view: bool,
) -> Result<Value> {
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
    serde_json::to_value(ProbePayload {
        selector,
        shadow_path: &target.shadow_path,
        observed_selectors,
        scroll_into_view,
    })
    .map_err(Into::into)
}

fn available_local_port() -> Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

async fn wait_until_ready(
    client: &Client,
    endpoint: &str,
    driver: &mut Child,
    kind: BrowserKind,
) -> Result<()> {
    for _ in 0..100 {
        if let Some(status) = driver.try_wait()? {
            bail!(
                "{} WebDriver 提前退出（{}）：{}",
                kind.display_name(),
                status,
                webdriver_session_hint(kind)
            );
        }
        if client
            .get(format!("{endpoint}/status"))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return Ok(());
        }
        sleep(Duration::from_millis(50)).await;
    }
    bail!(
        "{} WebDriver 启动超时：{}",
        kind.display_name(),
        webdriver_session_hint(kind)
    )
}

async fn parse_response(response: Response) -> Result<Value> {
    let status = response.status();
    let text = response.text().await?;
    let payload = if text.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str::<Value>(&text)
            .with_context(|| format!("WebDriver returned non-JSON data: {text}"))?
    };
    let value = payload
        .get("value")
        .cloned()
        .unwrap_or_else(|| payload.clone());
    let protocol_error = value.get("error").and_then(Value::as_str).map(|error| {
        let message = value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown WebDriver error");
        format!("{error}: {message}")
    });
    if !status.is_success() || protocol_error.is_some() {
        bail!(
            "WebDriver command failed ({}): {}",
            status,
            protocol_error.unwrap_or_else(|| text)
        );
    }
    Ok(value)
}

fn webdriver_missing_driver_message(kind: BrowserKind) -> String {
    match kind {
        BrowserKind::Safari => "未找到 macOS 自带的 safaridriver".into(),
        BrowserKind::Firefox => "未找到 geckodriver；Punctual 安装包应当自带该驱动".into(),
        _ => "未找到 WebDriver".into(),
    }
}

fn webdriver_session_hint(kind: BrowserKind) -> String {
    match kind {
        BrowserKind::Safari => concat!(
            "Safari 需要先允许远程自动化。可在 Safari 的“设置 → 高级”中显示开发菜单，",
            "再在“开发”菜单中启用“允许远程自动化”；Punctual 会在失败时自动尝试下一款浏览器"
        )
        .into(),
        BrowserKind::Firefox => {
            "请确认 Firefox 可以启动；Punctual 会使用独立用户目录并自动尝试下一款浏览器".into()
        }
        _ => "请检查浏览器自动化配置".into(),
    }
}

fn is_meaningful_result_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_webdriver_locator_payload() {
        let target = TargetFingerprint {
            role: "button".into(),
            accessible_name: "购买".into(),
            visible_text: "购买".into(),
            stable_attributes: Default::default(),
            context_text: None,
            selector_hint: Some("button.buy".into()),
            shadow_path: vec!["#host".into()],
            frame_path: Vec::new(),
        };
        let payload = locator_payload(&target).unwrap();
        assert_eq!(payload["selector"], "button.buy");
        assert_eq!(payload["shadowPath"][0], "#host");
    }
}
