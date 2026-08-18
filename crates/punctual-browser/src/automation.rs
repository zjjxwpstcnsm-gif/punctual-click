use std::{collections::BTreeSet, path::Path};

use anyhow::{bail, Result};
use chromiumoxide::Page;
use chrono::{DateTime, Utc};
use punctual_core::{ClickAttemptGuard, CompletionSignal, TargetCandidate, TargetFingerprint};
use url::Url;

use crate::{
    BrowserBackend, BrowserInstallation, ChromiumSession, CompletionBaseline,
    CompletionVerification, WebDriverPage, WebDriverSession,
};

#[derive(Clone)]
pub enum BrowserPage {
    Chromium(Page),
    WebDriver(WebDriverPage),
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetProbe {
    pub found: bool,
    pub clickable: bool,
    pub reason: Option<String>,
    #[serde(default)]
    pub page_url: Option<String>,
    #[serde(default)]
    pub visible_text: String,
    #[serde(default)]
    pub present_selectors: Vec<String>,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClickDispatch {
    pub dispatched_at: DateTime<Utc>,
    pub completion_baseline: CompletionBaseline,
    pub source_target_key: String,
    pub existing_target_keys: BTreeSet<String>,
    pub browser_name: String,
}

pub enum BrowserSession {
    Chromium(ChromiumSession),
    WebDriver(WebDriverSession),
}

impl BrowserSession {
    pub async fn launch(installation: &BrowserInstallation, profile_root: &Path) -> Result<Self> {
        let profile_dir = installation.profile_dir(profile_root);
        match installation.backend() {
            BrowserBackend::ChromiumCdp => {
                let executable = installation
                    .executable
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("Chromium 浏览器缺少可执行文件路径"))?;
                ChromiumSession::launch(profile_dir, Some(executable), installation.display_name())
                    .await
                    .map(Self::Chromium)
            }
            BrowserBackend::WebDriverSafari | BrowserBackend::WebDriverFirefox => {
                WebDriverSession::launch(installation, profile_dir)
                    .await
                    .map(Self::WebDriver)
            }
        }
    }

    pub fn browser_name(&self) -> &str {
        match self {
            Self::Chromium(session) => session.browser_name(),
            Self::WebDriver(session) => session.browser_name(),
        }
    }

    pub async fn open(&self, url: &Url) -> Result<BrowserPage> {
        match self {
            Self::Chromium(session) => session.open(url).await.map(BrowserPage::Chromium),
            Self::WebDriver(session) => session.open(url).await.map(BrowserPage::WebDriver),
        }
    }

    pub async fn detect_targets(&self, page: &BrowserPage) -> Result<Vec<TargetCandidate>> {
        match (self, page) {
            (Self::Chromium(session), BrowserPage::Chromium(page)) => {
                session.detect_targets(page).await
            }
            (Self::WebDriver(session), BrowserPage::WebDriver(page)) => {
                session.detect_targets(page).await
            }
            _ => bail!("浏览器页面与自动化会话类型不匹配"),
        }
    }

    pub async fn highlight(&self, page: &BrowserPage, target: &TargetFingerprint) -> Result<bool> {
        match (self, page) {
            (Self::Chromium(session), BrowserPage::Chromium(page)) => {
                session.highlight(page, target).await
            }
            (Self::WebDriver(session), BrowserPage::WebDriver(page)) => {
                session.highlight(page, target).await
            }
            _ => bail!("浏览器页面与自动化会话类型不匹配"),
        }
    }

    pub async fn prepare_target(
        &self,
        page: &BrowserPage,
        target: &TargetFingerprint,
    ) -> Result<TargetProbe> {
        match (self, page) {
            (Self::Chromium(session), BrowserPage::Chromium(page)) => {
                session.prepare_target(page, target).await
            }
            (Self::WebDriver(session), BrowserPage::WebDriver(page)) => {
                session.prepare_target(page, target).await
            }
            _ => bail!("浏览器页面与自动化会话类型不匹配"),
        }
    }

    pub async fn probe_target(
        &self,
        page: &BrowserPage,
        target: &TargetFingerprint,
    ) -> Result<TargetProbe> {
        match (self, page) {
            (Self::Chromium(session), BrowserPage::Chromium(page)) => {
                session.probe_target(page, target).await
            }
            (Self::WebDriver(session), BrowserPage::WebDriver(page)) => {
                session.probe_target(page, target).await
            }
            _ => bail!("浏览器页面与自动化会话类型不匹配"),
        }
    }

    pub async fn click_once(
        &self,
        page: &BrowserPage,
        target: &TargetFingerprint,
        guard: &ClickAttemptGuard,
        signals: &[CompletionSignal],
    ) -> Result<ClickDispatch> {
        match (self, page) {
            (Self::Chromium(session), BrowserPage::Chromium(page)) => {
                session.click_once(page, target, guard, signals).await
            }
            (Self::WebDriver(session), BrowserPage::WebDriver(page)) => {
                session.click_once(page, target, guard, signals).await
            }
            _ => bail!("浏览器页面与自动化会话类型不匹配"),
        }
    }

    pub async fn verify_completion(
        &self,
        page: &BrowserPage,
        dispatch: &ClickDispatch,
        signals: &[CompletionSignal],
    ) -> Result<CompletionVerification> {
        match (self, page) {
            (Self::Chromium(session), BrowserPage::Chromium(page)) => {
                session.verify_completion(page, dispatch, signals).await
            }
            (Self::WebDriver(session), BrowserPage::WebDriver(page)) => {
                session.verify_completion(page, dispatch, signals).await
            }
            _ => bail!("浏览器页面与自动化会话类型不匹配"),
        }
    }

    pub async fn current_url(&self, page: &BrowserPage) -> Result<Option<Url>> {
        match (self, page) {
            (Self::Chromium(_), BrowserPage::Chromium(page)) => match page.url().await? {
                Some(value) => Ok(Some(Url::parse(&value)?)),
                None => Ok(None),
            },
            (Self::WebDriver(session), BrowserPage::WebDriver(page)) => {
                session.current_url(page).await.map(Some)
            }
            _ => bail!("浏览器页面与自动化会话类型不匹配"),
        }
    }

    pub async fn close(self) -> Result<()> {
        match self {
            Self::Chromium(session) => session.close().await,
            Self::WebDriver(session) => session.close().await,
        }
    }
}
